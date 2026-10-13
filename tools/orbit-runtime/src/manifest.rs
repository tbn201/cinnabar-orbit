use serde::{Deserialize, Serialize};

pub const REPOSITORY: &str = "tbn201/cinnabar-orbit";
pub const MANIFEST_FILE: &str = "orbit-runtime.json";
pub const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const MAX_FILES: usize = 20_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub launcher_api: u32,
    pub version: String,
    pub commit: String,
    pub release: String,
    pub packages: Vec<Package>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub target: String,
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.launcher_api != 1 {
            return Err("This client release needs a newer OrBit Launcher.".into());
        }
        if self.commit.len() != 40
            || !self.commit.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.version.is_empty()
            || self.version.len() > 64
            || !self.release.starts_with("orbit-host-")
            || self.release.len() > 96
            || !self
                .release
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
            || self.packages.is_empty()
            || self.packages.len() > 3
        {
            return Err("The OrBit client release information is invalid.".into());
        }
        let mut targets = std::collections::BTreeSet::new();
        for package in &self.packages {
            if !matches!(
                package.target.as_str(),
                "aarch64-apple-darwin" | "x86_64-apple-darwin" | "x86_64-pc-windows-msvc"
            ) || !targets.insert(&package.target)
                || package.file != format!("orbit-runtime-{}.zip", package.target)
                || package.size == 0
                || package.size > MAX_ARCHIVE_BYTES
                || !digest_valid(&package.sha256)
            {
                return Err("The OrBit client download information is invalid.".into());
            }
        }
        Ok(())
    }

    pub fn package(&self, target: &str) -> Result<&Package, String> {
        self.packages
            .iter()
            .find(|package| package.target == target)
            .ok_or_else(|| {
                "There isn't an OrBit client build for this computer in this release.".into()
            })
    }

    pub fn directory(&self, target: &str) -> String {
        self.package(target)
            .map(|package| {
                format!(
                    "{}-{target}-{}",
                    &self.commit[..12].to_ascii_lowercase(),
                    &package.sha256[..12].to_ascii_lowercase()
                )
            })
            .unwrap_or_default()
    }

    pub fn download_url(&self, package: &Package) -> String {
        format!(
            "https://github.com/{REPOSITORY}/releases/download/{}/{}",
            self.release, package.file
        )
    }
}

pub fn digest_valid(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
