pub mod manifest;

use manifest::{Manifest, Package};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("package") if args.len() == 7 => package(Path::new(&args[1]), Path::new(&args[2]), &args[3], &args[4], &args[5], &args[6]),
        Some("manifest") if args.len() == 5 => publish(Path::new(&args[1]), &args[2], &args[3], &args[4]),
        _ => Err("usage: orbit-runtime package ROOT OUT TARGET COMMIT VERSION RELEASE | manifest OUT COMMIT VERSION RELEASE".into()),
    }
}

fn hash(path: &Path) -> Result<(u64, String), String> {
    let mut input = File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut size = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        size += count as u64;
        digest.update(&buffer[..count]);
    }
    Ok((size, format!("{:x}", digest.finalize())))
}

fn package(
    root: &Path,
    out: &Path,
    target: &str,
    commit: &str,
    version: &str,
    release: &str,
) -> Result<(), String> {
    fs::create_dir_all(out).map_err(|error| error.to_string())?;
    let file = format!("orbit-runtime-{target}.zip");
    let path = out.join(&file);
    let mut writer = zip::ZipWriter::new(File::create(&path).map_err(|error| error.to_string())?);
    let mut total = 0;
    let mut count = 0;
    walk(root, root, &mut writer, &mut total, &mut count)?;
    writer.finish().map_err(|error| error.to_string())?;
    let (size, sha256) = hash(&path)?;
    let package = Package {
        target: target.into(),
        file,
        size,
        sha256,
    };
    let manifest = Manifest {
        schema: 1,
        launcher_api: 1,
        version: version.into(),
        commit: commit.into(),
        release: release.into(),
        packages: vec![package],
    };
    manifest.validate()?;
    fs::write(
        out.join(format!("{target}.json")),
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn walk(
    root: &Path,
    directory: &Path,
    writer: &mut zip::ZipWriter<File>,
    total: &mut u64,
    count: &mut usize,
) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_dir() {
            walk(root, &entry.path(), writer, total, count)?;
            continue;
        }
        if !kind.is_file() {
            return Err(
                "Portable client payloads cannot contain symlinks or special files.".into(),
            );
        }
        *count += 1;
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        *total = total
            .checked_add(metadata.len())
            .ok_or("payload size overflow")?;
        if *count > manifest::MAX_FILES || *total > manifest::MAX_UNPACKED_BYTES {
            return Err("Portable client exceeds the launcher limits.".into());
        }
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o755
        };
        #[cfg(not(unix))]
        let mode = if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "exe")
        {
            0o755
        } else {
            0o644
        };
        let name = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        writer
            .start_file(name, SimpleFileOptions::default().unix_permissions(mode))
            .map_err(|error| error.to_string())?;
        let mut input = File::open(entry.path()).map_err(|error| error.to_string())?;
        std::io::copy(&mut input, writer).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn publish(out: &Path, commit: &str, version: &str, release: &str) -> Result<(), String> {
    let mut packages = Vec::new();
    for target in [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ] {
        let manifest: Manifest = serde_json::from_slice(
            &fs::read(out.join(format!("{target}.json"))).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        manifest.validate()?;
        if manifest.commit != commit || manifest.version != version || manifest.release != release {
            return Err("Platform builds do not describe the same release.".into());
        }
        let package = manifest.package(target)?;
        let (size, digest) = hash(&out.join(&package.file))?;
        if size != package.size || digest != package.sha256 {
            return Err("Portable client archive failed verification.".into());
        }
        packages.push(package.clone());
    }
    let manifest = Manifest {
        schema: 1,
        launcher_api: 1,
        commit: commit.into(),
        version: version.into(),
        release: release.into(),
        packages,
    };
    manifest.validate()?;
    fs::write(
        out.join(manifest::MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let mut sums = File::create(out.join("SHA256SUMS")).map_err(|error| error.to_string())?;
    for package in &manifest.packages {
        writeln!(sums, "{}  {}", package.sha256, package.file)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
