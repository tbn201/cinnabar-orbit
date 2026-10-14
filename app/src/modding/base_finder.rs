use chunk_pipeline::WorldStream;
use client_extensions::modding::orbit_base_finder::{Base, MAX_BASES, Policy};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};
use world::{BLOCKS_PER_SUB_CHUNK, SUB_CHUNK_SIDE, SubChunk, SubChunkKey};

const RADIUS_CHUNKS: i32 = 12;
const SUB_CHUNK_LAYERS: std::ops::RangeInclusive<i32> = -4..=19;
const PROBES_PER_FRAME: usize = 96;
const SCANS_PER_FRAME: usize = 4;
const RANK_INTERVAL_SECONDS: f64 = 0.5;
const DECOR_CAP: u32 = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Storage,
    Utility,
    Spawner,
    Decor,
}

const COLORS: [&str; 17] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray",
    "silver", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

const FIXED: [(&str, Kind, u8); 31] = [
    ("chest", Kind::Storage, 0),
    ("trapped_chest", Kind::Storage, 1),
    ("barrel", Kind::Storage, 2),
    ("shulker_box", Kind::Storage, 3),
    ("undyed_shulker_box", Kind::Storage, 3),
    ("ender_chest", Kind::Storage, 4),
    ("hopper", Kind::Storage, 5),
    ("dropper", Kind::Storage, 6),
    ("dispenser", Kind::Storage, 7),
    ("furnace", Kind::Utility, 8),
    ("lit_furnace", Kind::Utility, 8),
    ("blast_furnace", Kind::Utility, 9),
    ("lit_blast_furnace", Kind::Utility, 9),
    ("smoker", Kind::Utility, 10),
    ("lit_smoker", Kind::Utility, 10),
    ("crafting_table", Kind::Utility, 11),
    ("anvil", Kind::Utility, 12),
    ("chipped_anvil", Kind::Utility, 12),
    ("damaged_anvil", Kind::Utility, 12),
    ("enchanting_table", Kind::Utility, 13),
    ("brewing_stand", Kind::Utility, 14),
    ("bed", Kind::Utility, 15),
    ("beacon", Kind::Utility, 16),
    ("respawn_anchor", Kind::Utility, 17),
    ("mob_spawner", Kind::Spawner, 18),
    ("glass", Kind::Decor, 19),
    ("glass_pane", Kind::Decor, 19),
    ("hardened_clay", Kind::Decor, 23),
    ("quartz_block", Kind::Decor, 24),
    ("quartz_bricks", Kind::Decor, 24),
    ("smooth_quartz", Kind::Decor, 24),
];

const COLORED: [(&str, u8); 7] = [
    ("stained_glass", 19),
    ("stained_glass_pane", 19),
    ("concrete", 20),
    ("carpet", 21),
    ("wool", 22),
    ("terracotta", 23),
    ("glazed_terracotta", 23),
];

fn indicator_names() -> Vec<(String, Kind, u8)> {
    let fixed = FIXED
        .iter()
        .map(|(name, kind, group)| (format!("minecraft:{name}"), *kind, *group));
    let colored = COLORED.iter().flat_map(|(suffix, group)| {
        COLORS
            .iter()
            .map(move |color| (format!("minecraft:{color}_{suffix}"), Kind::Decor, *group))
    });
    let shulkers = COLORS
        .iter()
        .map(|color| (format!("minecraft:{color}_shulker_box"), Kind::Storage, 3));
    fixed.chain(colored).chain(shulkers).collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Tally {
    storage: u32,
    utility: u32,
    spawners: u32,
    decor: u32,
    groups: u64,
    key_sum: [f64; 3],
    decor_sum: [f64; 3],
}

impl Tally {
    fn add(&mut self, kind: Kind, group: u8, position: [i32; 3]) {
        let sum = if kind == Kind::Decor {
            self.decor += 1;
            &mut self.decor_sum
        } else {
            match kind {
                Kind::Storage => self.storage += 1,
                Kind::Utility => self.utility += 1,
                _ => self.spawners += 1,
            }
            &mut self.key_sum
        };
        for (total, value) in sum.iter_mut().zip(position) {
            *total += f64::from(value);
        }
        self.groups |= 1 << group;
    }

    fn merge(&mut self, other: &Self) {
        self.storage += other.storage;
        self.utility += other.utility;
        self.spawners += other.spawners;
        self.decor += other.decor;
        self.groups |= other.groups;
        for axis in 0..3 {
            self.key_sum[axis] += other.key_sum[axis];
            self.decor_sum[axis] += other.decor_sum[axis];
        }
    }

    fn key_blocks(&self) -> u32 {
        self.storage + self.utility + self.spawners
    }

    fn centre(&self) -> [f64; 3] {
        let (sum, count) = if self.key_blocks() > 0 {
            (self.key_sum, self.key_blocks())
        } else {
            (self.decor_sum, self.decor.max(1))
        };
        sum.map(|total| total / f64::from(count) + 0.5)
    }
}

fn score(tally: &Tally, columns: usize) -> Option<u32> {
    if tally.storage == 0 && tally.utility < 4 {
        return None;
    }
    if tally.spawners > 0 && tally.storage <= 2 && tally.utility == 0 {
        return None;
    }
    let raw = tally.storage * 6
        + tally.utility * 3
        + tally.spawners * 2
        + tally.decor.min(DECOR_CAP) / 4
        + tally.groups.count_ones() * 4;
    let sparse = tally.storage < 8 && (tally.storage as usize) < columns;
    Some(if sparse { raw / 3 } else { raw })
}

fn threshold(policy: Policy) -> u32 {
    70 - 6 * (policy.sensitivity.clamp(1.0, 10.0) as u32 - 1)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Ranked {
    base: Base,
    distance: f64,
}

fn rank(columns: &HashMap<[i32; 2], Tally>, origin: [f64; 3]) -> Vec<Ranked> {
    let mut seen = HashSet::new();
    let mut ranked = Vec::new();
    for start in columns.keys() {
        if !seen.insert(*start) {
            continue;
        }
        let mut cluster = Tally::default();
        let mut members = 0;
        let mut frontier = vec![*start];
        while let Some(column) = frontier.pop() {
            cluster.merge(&columns[&column]);
            members += 1;
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let next = [column[0] + dx, column[1] + dz];
                    if columns.contains_key(&next) && seen.insert(next) {
                        frontier.push(next);
                    }
                }
            }
        }
        let Some(score) = score(&cluster, members) else {
            continue;
        };
        let centre = cluster.centre();
        let base = Base {
            x: centre[0].floor().clamp(-30_000_000.0, 30_000_000.0) as i32,
            y: centre[1].floor().clamp(-64.0, 320.0) as i32,
            z: centre[2].floor().clamp(-30_000_000.0, 30_000_000.0) as i32,
            score: score.min(1_000_000),
            storage: cluster.storage.min(1_000_000),
        };
        let distance = (0..3)
            .map(|axis| (centre[axis] - origin[axis]).powi(2))
            .sum::<f64>();
        ranked.push(Ranked { base, distance });
    }
    ranked.sort_by(|a, b| {
        b.base
            .score
            .cmp(&a.base.score)
            .then(a.distance.total_cmp(&b.distance))
    });
    ranked
}

fn select(ranked: &[Ranked], policy: Policy) -> Vec<Base> {
    let minimum = threshold(policy);
    ranked
        .iter()
        .filter(|ranked| ranked.base.score >= minimum)
        .map(|ranked| ranked.base)
        .take(MAX_BASES)
        .collect()
}

struct Section {
    source: Weak<SubChunk>,
    tally: Tally,
}

#[derive(Default)]
pub(super) struct BaseFinder {
    identity: Option<(u64, usize, u64, assets::NetworkIdMode)>,
    lookup: HashMap<u32, (Kind, u8)>,
    centre: Option<(i32, [i32; 2])>,
    keys: Vec<SubChunkKey>,
    cursor: usize,
    sections: HashMap<SubChunkKey, Section>,
    dirty: bool,
    ranked_at: f64,
    ranked: Vec<Ranked>,
}

impl BaseFinder {
    pub(super) fn update(
        &mut self,
        stream: Option<&WorldStream>,
        feet: [f32; 3],
        policy: Option<Policy>,
        now: f64,
    ) -> Vec<Base> {
        let (Some(stream), Some(policy)) = (stream, policy) else {
            *self = Self::default();
            return Vec::new();
        };
        if !feet.iter().all(|axis| axis.is_finite()) {
            return Vec::new();
        }
        self.refresh_lookup(stream);
        let dimension = stream.current_dimension();
        let side = SUB_CHUNK_SIDE as i32;
        let chunk = [
            (feet[0].floor() as i32).div_euclid(side),
            (feet[2].floor() as i32).div_euclid(side),
        ];
        if self.centre != Some((dimension, chunk)) {
            self.recentre(dimension, chunk);
        }
        self.probe(stream.collision_store());
        if self.dirty && now >= self.ranked_at {
            self.dirty = false;
            self.ranked_at = now + RANK_INTERVAL_SECONDS;
            let mut columns: HashMap<[i32; 2], Tally> = HashMap::new();
            for (key, section) in &self.sections {
                if section.tally.key_blocks() > 0 || section.tally.decor > 0 {
                    columns
                        .entry([key.x, key.z])
                        .or_default()
                        .merge(&section.tally);
                }
            }
            columns.retain(|_, tally| tally.key_blocks() > 0);
            self.ranked = rank(&columns, feet.map(f64::from));
        }
        select(&self.ranked, policy)
    }

    fn refresh_lookup(&mut self, stream: &WorldStream) {
        let authority = stream.authority();
        let identity = (
            authority.actor_session_id(),
            Arc::as_ptr(stream.runtime_assets()) as usize,
            authority.block_name_revision(),
            stream.network_id_mode(),
        );
        if self.identity == Some(identity) {
            return;
        }
        *self = Self::default();
        self.identity = Some(identity);
        for (name, kind, group) in indicator_names() {
            for id in authority.block_ids_for_name(&name) {
                self.lookup.insert(*id, (kind, group));
            }
        }
    }

    fn recentre(&mut self, dimension: i32, chunk: [i32; 2]) {
        self.centre = Some((dimension, chunk));
        self.keys.clear();
        self.cursor = 0;
        for dx in -RADIUS_CHUNKS..=RADIUS_CHUNKS {
            for dz in -RADIUS_CHUNKS..=RADIUS_CHUNKS {
                for y in SUB_CHUNK_LAYERS {
                    self.keys
                        .push(SubChunkKey::new(dimension, chunk[0] + dx, y, chunk[1] + dz));
                }
            }
        }
        self.keys.sort_unstable_by_key(|key| {
            (
                (key.x - chunk[0]).pow(2) + (key.z - chunk[1]).pow(2),
                *key,
            )
        });
        let before = self.sections.len();
        self.sections.retain(|key, _| {
            key.dimension == dimension
                && (key.x - chunk[0]).abs() <= RADIUS_CHUNKS
                && (key.z - chunk[1]).abs() <= RADIUS_CHUNKS
        });
        self.dirty |= before != self.sections.len();
    }

    fn probe(&mut self, store: &world::ChunkStore) {
        if self.keys.is_empty() || self.lookup.is_empty() {
            return;
        }
        let mut scanned = 0;
        for _ in 0..PROBES_PER_FRAME.min(self.keys.len()) {
            let key = self.keys[self.cursor];
            let Some(source) = store.sub_chunk(key) else {
                self.dirty |= self.sections.remove(&key).is_some();
                self.cursor = (self.cursor + 1) % self.keys.len();
                continue;
            };
            let current = self
                .sections
                .get(&key)
                .and_then(|section| section.source.upgrade())
                .is_some_and(|cached| Arc::ptr_eq(&cached, &source));
            if !current {
                if scanned == SCANS_PER_FRAME {
                    return;
                }
                scanned += 1;
                let tally = tally_section(&source, key, &self.lookup);
                self.sections.insert(
                    key,
                    Section {
                        source: Arc::downgrade(&source),
                        tally,
                    },
                );
                self.dirty = true;
            }
            self.cursor = (self.cursor + 1) % self.keys.len();
        }
    }
}

fn tally_section(source: &SubChunk, key: SubChunkKey, lookup: &HashMap<u32, (Kind, u8)>) -> Tally {
    let mut tally = Tally::default();
    let Some(storage) = source.storages().first() else {
        return tally;
    };
    let palette: Vec<Option<(Kind, u8)>> = storage
        .palette()
        .values()
        .iter()
        .map(|id| lookup.get(id).copied())
        .collect();
    if palette.iter().all(Option::is_none) {
        return tally;
    }
    let mut indices = Box::new([0u32; BLOCKS_PER_SUB_CHUNK]);
    storage.palette_indices(&mut indices);
    let side = SUB_CHUNK_SIDE as i32;
    for (linear, index) in indices.iter().enumerate() {
        let Some(Some((kind, group))) = palette.get(*index as usize) else {
            continue;
        };
        let linear = linear as i32;
        let position = [
            key.x * side + (linear >> 8),
            key.y * side + (linear & 15),
            key.z * side + ((linear >> 4) & 15),
        ];
        tally.add(*kind, *group, position);
    }
    tally
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(storage: u32, utility: u32, spawners: u32, decor: u32, groups: &[u8]) -> Tally {
        let mut tally = Tally::default();
        let mut add = |kind, count| {
            for index in 0..count {
                tally.add(kind, groups[index as usize % groups.len()], [index as i32, -40, 0]);
            }
        };
        add(Kind::Storage, storage);
        add(Kind::Utility, utility);
        add(Kind::Spawner, spawners);
        add(Kind::Decor, decor);
        tally
    }

    #[test]
    fn natural_structures_are_suppressed_and_storage_heavy_bases_score() {
        assert_eq!(score(&tally(1, 0, 1, 0, &[0]), 1), None);
        assert_eq!(score(&tally(0, 0, 1, 0, &[18]), 1), None);
        assert!(score(&tally(1, 0, 0, 0, &[0]), 1).unwrap() < threshold(Policy { sensitivity: 10.0 }));
        assert_eq!(score(&tally(0, 2, 0, 40, &[11, 22]), 1), None);
        let base = score(&tally(12, 4, 0, 30, &[0, 2, 3, 8, 11, 22]), 2).unwrap();
        let village = score(&tally(3, 14, 0, 30, &[0, 8, 10, 11, 15, 21]), 18).unwrap();
        assert!(base > village * 3, "{base} {village}");
        assert!(base >= threshold(Policy { sensitivity: 1.0 }));
        assert!(village < threshold(Policy { sensitivity: 5.0 }));
    }

    #[test]
    fn sensitivity_lowers_the_threshold_monotonically() {
        let thresholds: Vec<_> = (1..=10)
            .map(|sensitivity| threshold(Policy { sensitivity: sensitivity as f32 }))
            .collect();
        assert!(thresholds.windows(2).all(|pair| pair[0] > pair[1]));
        assert_eq!(thresholds[9], 16);
    }

    #[test]
    fn neighbouring_columns_merge_and_results_are_best_then_nearest() {
        let mut columns = HashMap::new();
        columns.insert([0, 0], tally(6, 2, 0, 0, &[0, 8]));
        columns.insert([1, 1], tally(6, 1, 0, 0, &[2, 11]));
        columns.insert([10, 10], tally(4, 2, 0, 0, &[0, 8]));
        columns.insert([-10, -10], tally(4, 2, 0, 0, &[0, 8]));
        let ranked = rank(&columns, [100.0, -40.0, 0.0]);
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].base.storage, 12);
        assert!(ranked[1].distance <= ranked[2].distance);
        assert_eq!(ranked[0].base.y, -40);
        let chosen = select(&ranked, Policy { sensitivity: 10.0 });
        assert!(chosen.len() <= MAX_BASES && !chosen.is_empty());
        let values = chosen[0].values();
        assert_eq!(Base::parse(&values), Some(chosen[0]));
    }

    #[test]
    fn indicator_names_cover_storage_colours_and_are_unique() {
        let names = indicator_names();
        let unique: HashSet<_> = names.iter().map(|(name, _, _)| name.clone()).collect();
        assert_eq!(unique.len(), names.len());
        assert!(unique.contains("minecraft:red_shulker_box"));
        assert!(unique.contains("minecraft:light_blue_concrete"));
        assert!(names.iter().all(|(_, _, group)| *group < 64));
    }
}
