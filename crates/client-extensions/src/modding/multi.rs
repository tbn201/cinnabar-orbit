//! Several local mods in one frame. Load order resolves every conflict.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use {
    mod_api::MAX_LOADED_MODS,
    mod_host::{
        CameraDelta, ControlFrame, GameplayCameraRig, GameplayMob, GameplayMovementSnapshot,
        GameplaySnapshot, ModCue, ModGrants, ModHost, PlayerStateSnapshot,
    },
};

/// Loaded personal components and their merged labels, in load order.
pub struct ModHosts {
    pub host: ModHost,
    pub companions: Vec<Companion>,
    panel_selection: usize,
    label: Option<String>,
    label_inputs: Vec<String>,
    label_rebuilds: u64,
}

/// Selects an ordered set of components and packages, each with its own grants.
pub const SET_ENV: &str = "CINNABAR_MOD_SET";
const MAX_SET_BYTES: usize = 16 * 1024;

/// A mod after the first, loaded only from an explicit set.
pub struct Companion {
    pub host: ModHost,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModSet {
    version: u32,
    mods: Vec<SetEntry>,
}

/// One set entry names a bare component or a package directory, never both.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetEntry {
    #[serde(default)]
    component: Option<PathBuf>,
    #[serde(default)]
    package: Option<PathBuf>,
    #[serde(default)]
    grants: ModGrants,
}

/// Where a set entry's mod comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModSource {
    Component(PathBuf),
    /// A package's permissions need both its manifest's ask and the entry's grant.
    Package(PathBuf),
}

impl ModSource {
    /// Returns the component or package path selected by this entry.
    pub fn path(&self) -> &Path {
        match self {
            Self::Component(path) | Self::Package(path) => path,
        }
    }

    /// Loads this entry with only its explicitly supplied grants.
    pub fn load(&self, grants: ModGrants) -> anyhow::Result<ModHost> {
        match self {
            Self::Component(path) => ModHost::load_with_grants(path, grants),
            Self::Package(dir) => ModHost::load_package_with_grants(dir, grants),
        }
    }
}

/// Reads a bounded set file; its order is the load order.
pub fn read_set(path: &Path) -> Result<Vec<(ModSource, ModGrants)>, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| {
            use std::io::Read;
            file.take(MAX_SET_BYTES as u64 + 1).read_to_end(&mut bytes)
        })
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    if bytes.len() > MAX_SET_BYTES {
        return Err("mod set exceeds its byte limit".into());
    }
    let set: ModSet =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid mod set: {error}"))?;
    if set.version != 1 {
        return Err("unsupported mod set version".into());
    }
    if set.mods.is_empty() || set.mods.len() > MAX_LOADED_MODS {
        return Err(format!("a mod set lists 1 to {MAX_LOADED_MODS} components"));
    }
    set.mods
        .into_iter()
        .map(|entry| {
            let source = match (entry.component, entry.package) {
                (Some(path), None) => ModSource::Component(path),
                (None, Some(dir)) => ModSource::Package(dir),
                _ => return Err("a mod set entry names one component or one package".into()),
            };
            if !source.path().is_absolute() {
                return Err(format!(
                    "mod set entry {} must be absolute",
                    source.path().display()
                ));
            }
            Ok((source, entry.grants))
        })
        .collect()
}

/// Loads valid entries in order and reports failures without discarding the other entries.
pub fn load_entries(mods: Vec<(ModSource, ModGrants)>) -> Vec<ModHost> {
    mods.into_iter()
        .filter_map(|(source, grants)| {
            source
                .load(grants)
                .inspect_err(|error| {
                    let path = source.path().display();
                    eprintln!("Cinnabar extension {path} disabled: {error:#}");
                })
                .ok()
        })
        .collect()
}

impl ModHosts {
    /// Keeps the first host and its companions in their original load order.
    pub fn new(host: ModHost, companions: Vec<Companion>) -> Self {
        Self {
            host,
            companions,
            panel_selection: 0,
            label: None,
            label_inputs: Vec::new(),
            label_rebuilds: 0,
        }
    }

    /// Counts the primary component and every companion.
    pub fn host_count(&self) -> usize {
        1 + self.companions.len()
    }

    /// Index 0 is the first-loaded mod; companions follow in load order.
    pub fn host(&self, index: usize) -> &ModHost {
        match index {
            0 => &self.host,
            _ => &self.companions[index - 1].host,
        }
    }

    /// Borrows a component in load order for one callback or host action.
    pub fn host_mut(&mut self, index: usize) -> &mut ModHost {
        match index {
            0 => &mut self.host,
            _ => &mut self.companions[index - 1].host,
        }
    }

    /// The selected panel publisher owns the retained panel and its events.
    pub fn panel_owner(&self) -> usize {
        if self.panel_selection < self.host_count()
            && self.host(self.panel_selection).panel().is_some()
        {
            return self.panel_selection;
        }
        (0..self.host_count())
            .find(|&index| self.host(index).panel().is_some())
            .unwrap_or(0)
    }

    /// Selects the earliest active panel whose toggle key was pressed, closing the old panel.
    pub fn select_panel(&mut self, pressed: &[String]) -> bool {
        let current = self.host(self.panel_owner());
        if current.panel_open() && current.panel().is_some_and(|panel| panel.capture_key) {
            return false;
        }
        let selected = (0..self.host_count()).find(|&index| {
            let host = self.host(index);
            host.is_active()
                && host
                    .panel()
                    .is_some_and(|panel| pressed.contains(&panel.toggle_key))
        });
        let Some(selected) = selected else {
            return false;
        };
        let previous = self.panel_owner();
        self.panel_selection = selected;
        if selected != previous {
            self.host_mut(previous).set_panel_open(false);
            return true;
        }
        false
    }

    /// Removes a companion without changing the remaining components' order.
    pub fn remove_companion(&mut self, index: usize) -> ModHost {
        assert!(index > 0);
        let host = self.companions.remove(index - 1).host;
        if self.panel_selection == index {
            self.panel_selection = 0;
        } else if self.panel_selection > index {
            self.panel_selection -= 1;
        }
        host
    }

    /// The earliest card publisher owns the retained card surface.
    pub fn hud_owner(&self) -> Option<usize> {
        (0..self.host_count()).find(|&index| self.host(index).hud().is_some())
    }

    /// The earliest cursor publisher owns the retained cursor replacement.
    pub fn crosshair_owner(&self) -> Option<usize> {
        (0..self.host_count()).find(|&index| self.host(index).crosshair().is_some())
    }

    /// The mod whose package draws beside the container screens and receives their input.
    pub fn screen_owner(&self) -> Option<usize> {
        screen_owner((0..self.host_count()).map(|index| {
            let host = self.host(index);
            let screens = host.screens();
            (
                host.package().is_some() && host.is_active() && host.grants().screen,
                screens.overlay.is_some() || screens.view.is_some(),
            )
        }))
    }

    /// Every mod's label in load order, joined and cut to the plain-text limit.
    pub fn merged_label(&mut self) -> Option<&str> {
        if self.host_count() == 1 {
            return self.host.label();
        }
        let mut current = (0..self.host_count()).filter_map(|index| self.host(index).label());
        let unchanged = self
            .label_inputs
            .iter()
            .map(String::as_str)
            .eq(&mut current);
        if !unchanged {
            self.label_inputs = (0..self.host_count())
                .filter_map(|index| self.host(index).label().map(str::to_owned))
                .collect();
            self.label = join_labels(&self.label_inputs);
            self.label_rebuilds += 1;
        }
        self.label.as_deref()
    }

    /// Every mod's render output with its generation, in load order.
    pub fn render_outputs(&self) -> impl Iterator<Item = (&mod_render::RenderOutput, u64)> {
        (0..self.host_count()).map(|index| self.host(index).render())
    }

    /// Collects the distinct physical keys reserved by all loaded components.
    pub fn reserved_keys(&self) -> Vec<String> {
        let mut keys: Vec<_> = (0..self.host_count())
            .flat_map(|index| self.host(index).reserved_keys().iter().cloned())
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }
}

/// The controls one mod sees: keys reserved by an earlier mod are withheld, and only the
/// panel owner receives panel events.
fn claim_controls(frame: &ControlFrame, claimed: &[String], panel_owner: bool) -> ControlFrame {
    let free = |keys: &[String]| -> Vec<String> {
        keys.iter()
            .filter(|key| !claimed.contains(key))
            .cloned()
            .collect()
    };
    ControlFrame {
        keys_pressed: free(&frame.keys_pressed),
        keys_held: free(&frame.keys_held),
        events: if panel_owner {
            frame.events.clone()
        } else {
            Vec::new()
        },
        ..frame.clone()
    }
}

/// Host-owned input shared by every mod in one frame.
pub struct FrameInput<'a> {
    pub pressed: bool,
    pub controls: &'a ControlFrame,
    /// Every mod's committed cues from the previous frame.
    pub previous_cues: &'a [ModCue],
    /// Captured once from the current local session, delivered only to granted components.
    pub player_state: Option<&'a PlayerStateSnapshot>,
    pub world_observation: Option<&'a mod_host::WorldObservation>,
    pub modules: Option<&'a mod_host::module_observations::ModuleObservations>,
    pub fps: Option<f32>,
    pub sidebar: Option<&'a mod_host::SidebarSnapshot>,
    pub chat: &'a [mod_host::ObservedChatMessage],
}

/// Runs each mod once in load order with its own grants and merges what they commit.
/// A failing mod is reported and quarantined by its host; the others still run.
pub fn run_frame(
    runtime: &mut ModHosts,
    input: FrameInput<'_>,
    mut world: impl FnMut(
        &ModGrants,
    ) -> (
        Option<GameplaySnapshot>,
        Vec<GameplayMob>,
        Option<GameplayMovementSnapshot>,
    ),
    mut failed: impl FnMut(usize, String),
) -> Merged {
    let mut union = ModGrants::default();
    let mut active = false;
    for index in 0..runtime.host_count() {
        let host = runtime.host(index);
        if !host.is_active() {
            continue;
        }
        active = true;
        let grants = host.grants();
        union.players |= grants.players;
        union.camera |= grants.camera;
        union.item_use |= grants.item_use;
        union.movement |= grants.movement;
        union.interaction |= grants.interaction;
        union.entities |= grants.entities;
        union.commands.extend(grants.commands.iter().cloned());
    }
    let (snapshot, mobs, movement) = if active {
        world(&union)
    } else {
        (None, Vec::new(), None)
    };
    let owner = runtime.panel_owner();
    let mut claimed = Vec::new();
    let mut merged = Merged::default();
    for index in 0..runtime.host_count() {
        let host = runtime.host(index);
        if !host.is_active() {
            continue;
        }
        let grants = host.grants();
        let snapshot = snapshot
            .as_ref()
            .filter(|_| needs_gameplay(grants))
            .map(|frame| GameplaySnapshot {
                players: if grants.players {
                    frame.players.clone()
                } else {
                    Vec::new()
                },
                session: frame.session,
                dimension: frame.dimension,
                eye: frame.eye,
                yaw: frame.yaw,
                pitch: frame.pitch,
                frame_seconds: frame.frame_seconds,
                attack_held: frame.attack_held,
            });
        let mobs = if grants.entities {
            mobs.clone()
        } else {
            Vec::new()
        };
        let movement = if grants.movement { movement } else { None };
        let mut controls = claim_controls(input.controls, &claimed, index == owner);
        controls.panel_open = input.controls.panel_open && index == owner;
        controls.gameplay = snapshot.is_some() || movement.is_some();
        claimed.extend(runtime.host(index).reserved_keys().iter().cloned());
        let host = runtime.host_mut(index);
        let available = assist_capabilities(host.grants());
        let extended = extended_capabilities(host.grants());
        let incoming = input
            .previous_cues
            .iter()
            .filter(|cue| {
                !matches!(
                    cue.name.as_str(),
                    super::orbit_pings::HEADER | super::orbit_pings::ROW
                ) || host.grants().players
            })
            .filter(|cue| {
                cue.name != super::orbit_base_finder::BASE
                    || extended & super::orbit_base_finder::CAPABILITY != 0
            })
            .cloned()
            .map(|mut cue| {
                if let Some(values) = extended_ready_values(&cue.name, extended) {
                    cue.values = values;
                }
                if cue.name == super::orbit_aim_assist::READY {
                    cue.values = vec![1.0, u8::from(aim_capability(host.grants())) as f32];
                }
                if cue.name == super::orbit_autominer::READY {
                    cue.values = vec![1.0, u8::from(available & super::orbit_autominer::CAPABILITY != 0) as f32];
                }
                if cue.name == super::orbit_autoclicker::READY {
                    cue.values = vec![1.0, u8::from(available & super::orbit_autoclicker::CAPABILITY != 0) as f32];
                }
                if cue.name == super::orbit_hitbox::READY {
                    cue.values = vec![
                        1.0,
                        u8::from(available & super::orbit_hitbox::CAPABILITY != 0) as f32,
                    ];
                }
                if cue.name == super::orbit_protocol::READY {
                    cue.values = super::orbit_protocol::ready_values(
                        super::orbit_protocol::capabilities(&cue.values) & available,
                    )
                    .to_vec();
                }
                if cue.name == super::orbit_protocol::AUTO_SPRINT_READY {
                    cue.values = vec![
                        1.0,
                        f32::from(u8::from(
                            available & super::orbit_protocol::AUTO_SPRINT_CAPABILITY != 0
                                && super::orbit_protocol::auto_sprint_requested(&cue.values),
                        )),
                    ];
                }
                cue
            })
            .collect();
        host.deliver_cues(incoming);
        let player_state = host
            .grants()
            .player_state
            .then(|| input.player_state.cloned())
            .flatten();
        let observed = host.observe_presentation(
            input.world_observation.cloned(),
            input.fps,
            input.sidebar.cloned(),
            input.chat.to_vec(),
        );
        let observed = observed
            .and_then(|()| host.observe_modules(input.modules.cloned().unwrap_or_default()));
        if let Err(error) = observed {
            failed(index, format!("{error:#}"));
            continue;
        }
        if let Err(error) = host.frame_with_movement(
            input.pressed,
            snapshot,
            mobs,
            player_state,
            movement,
            controls,
        ) {
            failed(index, format!("{error:#}"));
        }
        if let Some(error) = host.take_settings_error() {
            eprintln!("Cinnabar extension preferences could not be saved: {error}");
        }
        merged.absorb(host);
    }
    merged
}

/// Whether the host may receive the common gameplay pose and input snapshot.
fn needs_gameplay(grants: &ModGrants) -> bool {
    grants.players
        || grants.camera
        || grants.item_use
        || grants.movement
        || grants.interaction
        || grants.entities
        || !grants.commands.is_empty()
}

/// One frame's combined output; for single-valued outputs the earliest mod wins.
#[derive(Default)]
pub struct Merged {
    pub rig: Option<GameplayCameraRig>,
    pub preserve_teleport_rotation: bool,
    pub view_scale: Option<[f32; 2]>,
    pub item_use_delay_fix: Option<(u64, i32)>,
    pub delta: Option<CameraDelta>,
    pub time_override: Option<u32>,
    pub attack_reach: Option<f32>,
    pub attack_pulse: bool,
    pub jump_pulse: bool,
    pub jump_cancel: bool,
    pub commands: Vec<String>,
    pub cues: Vec<ModCue>,
    pub assist_capabilities: u8,
    pub auto_hotbar: Option<(bool, bool)>,
    pub toggle_sprint: bool,
    pub auto_sprint: bool,
    pub aim_assist: Option<super::orbit_aim_assist::Policy>,
    pub aim_ready: bool,
    pub autominer: Option<super::orbit_autominer::Policy>,
    pub autoclicker: Option<super::orbit_autoclicker::Policy>,
    pub debounce_ms: Option<f32>,
    pub open_closet: bool,
    pub hitbox: Option<super::orbit_hitbox::HitboxPolicy>,
    pub extended_capabilities: u16,
    pub auto_totem: Option<super::orbit_auto_totem::Policy>,
    pub freecam: Option<super::orbit_freecam::Policy>,
    pub fly: Option<super::orbit_fly::Policy>,
    pub no_fall: bool,
    pub spider: bool,
    pub step: Option<super::orbit_step::Policy>,
    pub base_finder: Option<super::orbit_base_finder::Policy>,
    pub first_person_rig: bool,
}

impl Merged {
    /// Consumes `host`'s committed output, in load order.
    pub fn absorb(&mut self, host: &mut ModHost) {
        self.jump_pulse |= host.take_jump_pulse();
        self.jump_cancel |= host.take_jump_cancel();
        let interaction = host.take_interaction();
        self.attack_reach = self.attack_reach.or(interaction.attack_reach);
        self.attack_pulse |= interaction.attack_pulse;
        let delta = host.take_camera_delta();
        self.delta = self.delta.or(delta);
        let owns_rig = self.rig.is_none() && host.camera_rig().is_some();
        self.rig = self.rig.or(host.camera_rig());
        self.preserve_teleport_rotation |= host.preserves_teleport_rotation();
        self.view_scale = self.view_scale.or(host.camera_view_scale());
        self.item_use_delay_fix = self.item_use_delay_fix.or(host.item_use_delay_fix());
        self.time_override = self.time_override.or(host.time_override());
        self.commands.extend(host.take_commands());
        use super::orbit_protocol::{self as protocol, Policy};
        let available = if host.is_active() {
            assist_capabilities(host.grants())
        } else {
            0
        };
        let controls = available & protocol::CLOSET != 0;
        let hotbar = available & protocol::HOTBAR != 0;
        let sprint = available & protocol::SPRINT != 0;
        self.assist_capabilities |= available;
        let aim_ready = host.is_active() && aim_capability(host.grants());
        self.aim_ready |= aim_ready;
        let extended = if host.is_active() {
            extended_capabilities(host.grants())
        } else {
            0
        };
        self.extended_capabilities |= extended;
        for cue in host.take_cues() {
            if self.absorb_extended(&cue, extended, owns_rig) {
                continue;
            }
            match cue.name.as_str() {
                protocol::POLICY => {
                    if let Some(policy) = Policy::parse(&cue.values) {
                        if hotbar && (policy.auto_stack || policy.auto_tool) {
                            self.auto_hotbar
                                .get_or_insert((policy.auto_stack, policy.auto_tool));
                        }
                        self.toggle_sprint |= sprint && policy.toggle_sprint;
                    }
                }
                super::orbit_aim_assist::REQUEST => {
                    if aim_ready {
                        if let Some(policy) = super::orbit_aim_assist::Policy::parse(&cue.values) {
                            self.aim_assist.get_or_insert(policy);
                        }
                    }
                }
                super::orbit_aim_assist::READY => {}
                super::orbit_autominer::REQUEST => {
                    if available & super::orbit_autominer::CAPABILITY != 0 {
                        if let Some(policy) = super::orbit_autominer::Policy::parse(&cue.values) {
                            self.autominer.get_or_insert(policy);
                        }
                    }
                }
                super::orbit_autominer::READY => {}
                super::orbit_autoclicker::REQUEST => {
                    if available & super::orbit_autoclicker::CAPABILITY != 0 {
                        if let Some(policy) = super::orbit_autoclicker::Policy::parse(&cue.values) {
                            self.autoclicker.get_or_insert(policy);
                        }
                    }
                }
                super::orbit_autoclicker::READY => {}
                protocol::AUTO_SPRINT => {
                    self.auto_sprint |= available & protocol::AUTO_SPRINT_CAPABILITY != 0
                        && protocol::auto_sprint_requested(&cue.values);
                }
                protocol::DC_PREVENT => {
                    if available & protocol::DEBOUNCE != 0 {
                        if let Some(delay) = protocol::debounce_delay(&cue.values) {
                            self.debounce_ms.get_or_insert(delay);
                        }
                    }
                }
                super::orbit_hitbox::REQUEST => {
                    if available & super::orbit_hitbox::CAPABILITY != 0 {
                        if let Some(policy) = super::orbit_hitbox::HitboxPolicy::parse(&cue.values)
                        {
                            self.hitbox.get_or_insert(policy);
                        }
                    }
                }
                super::orbit_hitbox::READY => {}
                protocol::COSMETICS => self.open_closet |= controls && cue.values == [1.0],
                protocol::READY
                | protocol::AUTO_SPRINT_READY
                | super::orbit_pings::HEADER
                | super::orbit_pings::ROW => {}
                _ => self.cues.push(cue),
            }
        }
    }
}

impl Merged {
    fn absorb_extended(&mut self, cue: &ModCue, extended: u16, owns_rig: bool) -> bool {
        use super::{
            orbit_auto_totem as totem, orbit_base_finder as base, orbit_camera_rig as rig,
            orbit_fly as fly, orbit_freecam as freecam, orbit_no_fall as no_fall,
            orbit_spider as spider, orbit_step as step,
        };
        let granted = |bit: u16| extended & bit != 0;
        match cue.name.as_str() {
            totem::REQUEST if granted(totem::CAPABILITY) => {
                if let Some(policy) = totem::Policy::parse(&cue.values) {
                    self.auto_totem.get_or_insert(policy);
                }
            }
            freecam::REQUEST if granted(freecam::CAPABILITY) => {
                if let Some(policy) = freecam::Policy::parse(&cue.values) {
                    self.freecam.get_or_insert(policy);
                }
            }
            fly::REQUEST if granted(fly::CAPABILITY) => {
                if let Some(policy) = fly::Policy::parse(&cue.values) {
                    self.fly.get_or_insert(policy);
                }
            }
            no_fall::REQUEST if granted(no_fall::CAPABILITY) => {
                self.no_fall |= no_fall::requested(&cue.values);
            }
            spider::REQUEST if granted(spider::CAPABILITY) => {
                self.spider |= spider::requested(&cue.values);
            }
            step::REQUEST if granted(step::CAPABILITY) => {
                if let Some(policy) = step::Policy::parse(&cue.values) {
                    self.step.get_or_insert(policy);
                }
            }
            base::REQUEST if granted(base::CAPABILITY) => {
                if let Some(policy) = base::Policy::parse(&cue.values) {
                    self.base_finder.get_or_insert(policy);
                }
            }
            rig::REQUEST if granted(rig::CAPABILITY) => {
                self.first_person_rig |= owns_rig && rig::requested(&cue.values);
            }
            totem::REQUEST | freecam::REQUEST | fly::REQUEST | no_fall::REQUEST
            | spider::REQUEST | step::REQUEST | base::REQUEST | base::BASE | rig::REQUEST => {}
            name => return extended_ready_values(name, u16::MAX).is_some(),
        }
        true
    }
}

pub const EXTENDED_READY: [(&str, u16); 8] = [
    (super::orbit_auto_totem::READY, super::orbit_auto_totem::CAPABILITY),
    (super::orbit_freecam::READY, super::orbit_freecam::CAPABILITY),
    (super::orbit_fly::READY, super::orbit_fly::CAPABILITY),
    (super::orbit_no_fall::READY, super::orbit_no_fall::CAPABILITY),
    (super::orbit_spider::READY, super::orbit_spider::CAPABILITY),
    (super::orbit_step::READY, super::orbit_step::CAPABILITY),
    (super::orbit_base_finder::READY, super::orbit_base_finder::CAPABILITY),
    (super::orbit_camera_rig::READY, super::orbit_camera_rig::CAPABILITY),
];

pub fn extended_ready_values(name: &str, capabilities: u16) -> Option<Vec<f32>> {
    EXTENDED_READY
        .iter()
        .find(|(ready, _)| *ready == name)
        .map(|(_, bit)| vec![1.0, f32::from(u8::from(capabilities & bit != 0))])
}

pub fn extended_capabilities(grants: &ModGrants) -> u16 {
    let bit = |granted: bool, capability: u16| u16::from(granted) * capability;
    bit(
        grants.controls && grants.player_state && grants.item_use,
        super::orbit_auto_totem::CAPABILITY,
    ) | bit(
        grants.camera && grants.controls && grants.movement,
        super::orbit_freecam::CAPABILITY,
    ) | bit(
        grants.movement && grants.controls,
        super::orbit_fly::CAPABILITY,
    ) | bit(grants.movement, super::orbit_no_fall::CAPABILITY)
        | bit(
            grants.movement && grants.controls,
            super::orbit_spider::CAPABILITY,
        )
        | bit(grants.movement, super::orbit_step::CAPABILITY)
        | bit(
            grants.player_state && grants.render,
            super::orbit_base_finder::CAPABILITY,
        )
        | bit(grants.camera, super::orbit_camera_rig::CAPABILITY)
}

fn aim_capability(grants: &ModGrants) -> bool {
    grants.controls && grants.camera && grants.players && grants.entities
}

fn assist_capabilities(grants: &ModGrants) -> u8 {
    use super::orbit_protocol::{AUTO_SPRINT_CAPABILITY, CLOSET, DEBOUNCE, HOTBAR, SPRINT};
    if !grants.controls {
        return 0;
    }
    CLOSET
        | (u8::from(grants.interaction && grants.camera && grants.player_state && grants.item_use) * super::orbit_autominer::CAPABILITY)
        | (u8::from(grants.interaction) * super::orbit_autoclicker::CAPABILITY)
        | DEBOUNCE
        | (u8::from(grants.player_state && grants.item_use) * HOTBAR)
        | (u8::from(grants.movement) * (SPRINT | AUTO_SPRINT_CAPABILITY))
        | (u8::from(
            grants.player_state
                && grants.players
                && grants.entities
                && grants.interaction
                && grants.render,
        ) * super::orbit_hitbox::CAPABILITY)
}

/// Of mods in load order, each `(may draw, draws)`: the earliest that draws an overlay or view,
/// else the earliest that may draw one.
fn screen_owner(mods: impl Iterator<Item = (bool, bool)> + Clone) -> Option<usize> {
    let eligible = mods.enumerate().filter(|(_, (may, _))| *may);
    eligible
        .clone()
        .find(|(_, (_, draws))| *draws)
        .or_else(|| eligible.clone().next())
        .map(|(index, _)| index)
}

/// Joins labels with a separator, cut at a char boundary within the plain-text limit.
fn join_labels(labels: &[String]) -> Option<String> {
    if labels.is_empty() {
        return None;
    }
    let mut label = labels.join(" | ");
    if label.len() > mod_host::MAX_LABEL_BYTES {
        let mut end = mod_host::MAX_LABEL_BYTES;
        while !label.is_char_boundary(end) {
            end -= 1;
        }
        label.truncate(end);
    }
    Some(label)
}

#[cfg(test)]
#[path = "multi_tests.rs"]
mod tests;

#[cfg(test)]
mod assist_tests {
    use super::super::orbit_protocol::{AUTO_SPRINT_CAPABILITY, CLOSET, DEBOUNCE, HOTBAR, SPRINT};
    use super::*;

    #[test]
    fn readiness_is_scoped_to_the_receiving_components_grants() {
        let mut grants = ModGrants::default();
        assert_eq!(assist_capabilities(&grants), 0);
        grants.controls = true;
        assert_eq!(assist_capabilities(&grants), CLOSET | DEBOUNCE);
        grants.player_state = true;
        assert_eq!(assist_capabilities(&grants), CLOSET | DEBOUNCE);
        grants.item_use = true;
        assert_eq!(assist_capabilities(&grants), CLOSET | DEBOUNCE | HOTBAR);
        grants.movement = true;
        assert_eq!(
            assist_capabilities(&grants),
            CLOSET | DEBOUNCE | HOTBAR | SPRINT | AUTO_SPRINT_CAPABILITY
        );
        grants.controls = false;
        assert_eq!(assist_capabilities(&grants), 0);
    }
    #[test]
    fn aim_assist_requires_camera_and_both_actor_observation_grants() {
        let granted = ModGrants { controls:true, camera:true, players:true, entities:true, ..Default::default() };
        assert!(aim_capability(&granted));
        for field in 0..4 {
            let mut missing = granted.clone();
            match field { 0 => missing.controls=false, 1 => missing.camera=false,
                2 => missing.players=false, _ => missing.entities=false }
            assert!(!aim_capability(&missing));
        }
    }
    #[test]
    fn autominer_requires_controls_camera_interaction_and_item_authority() {
        let granted = ModGrants { controls: true, camera: true, interaction: true,
            player_state: true, item_use: true, ..ModGrants::default() };
        let bit = super::super::orbit_autominer::CAPABILITY;
        assert_ne!(assist_capabilities(&granted) & bit, 0);
        for field in 0..5 {
            let mut missing = granted.clone();
            match field { 0 => missing.controls = false, 1 => missing.camera = false,
                2 => missing.interaction = false, 3 => missing.player_state = false, _ => missing.item_use = false }
            assert_eq!(assist_capabilities(&missing) & bit, 0);
        }
    }
    #[test]
    fn autoclicker_requires_both_controls_and_interaction_authority() {
        for controls in [false, true] {
            for interaction in [false, true] {
                let grants = ModGrants { controls, interaction, ..ModGrants::default() };
                assert_eq!(assist_capabilities(&grants) & super::super::orbit_autoclicker::CAPABILITY != 0,
                    controls && interaction);
            }
        }
    }
    #[test]
    fn extended_features_require_exactly_their_grants() {
        use super::super::{
            orbit_auto_totem as totem, orbit_base_finder as base, orbit_fly as fly,
            orbit_freecam as freecam, orbit_no_fall as no_fall, orbit_spider as spider,
            orbit_step as step,
        };
        let cases: [(u16, ModGrants); 8] = [
            (totem::CAPABILITY, ModGrants { controls: true, player_state: true, item_use: true, ..ModGrants::default() }),
            (freecam::CAPABILITY, ModGrants { camera: true, controls: true, movement: true, ..ModGrants::default() }),
            (fly::CAPABILITY, ModGrants { movement: true, controls: true, ..ModGrants::default() }),
            (no_fall::CAPABILITY, ModGrants { movement: true, ..ModGrants::default() }),
            (spider::CAPABILITY, ModGrants { movement: true, controls: true, ..ModGrants::default() }),
            (step::CAPABILITY, ModGrants { movement: true, ..ModGrants::default() }),
            (base::CAPABILITY, ModGrants { player_state: true, render: true, ..ModGrants::default() }),
            (super::super::orbit_camera_rig::CAPABILITY, ModGrants { camera: true, ..ModGrants::default() }),
        ];
        for (bit, granted) in cases {
            assert_ne!(extended_capabilities(&granted) & bit, 0);
            let fields = [
                granted.controls, granted.player_state, granted.item_use, granted.camera,
                granted.movement, granted.render,
            ];
            for field in (0..fields.len()).filter(|field| fields[*field]) {
                let mut missing = granted.clone();
                match field {
                    0 => missing.controls = false,
                    1 => missing.player_state = false,
                    2 => missing.item_use = false,
                    3 => missing.camera = false,
                    4 => missing.movement = false,
                    _ => missing.render = false,
                }
                assert_eq!(extended_capabilities(&missing) & bit, 0);
            }
        }
        assert_eq!(extended_capabilities(&ModGrants::default()), 0);
        let mut bits = EXTENDED_READY.map(|(_, bit)| bit);
        bits.sort_unstable();
        assert!(bits.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(bits.iter().all(|bit| bit.count_ones() == 1 && *bit > 255));
    }
    #[test]
    fn extended_readiness_is_filtered_per_receiver() {
        use super::super::orbit_fly as fly;
        assert_eq!(extended_ready_values(fly::READY, fly::CAPABILITY), Some(vec![1.0, 1.0]));
        assert_eq!(extended_ready_values(fly::READY, 0), Some(vec![1.0, 0.0]));
        assert_eq!(extended_ready_values(fly::REQUEST, u16::MAX), None);
    }
    #[test]
    fn extended_requests_need_grants_and_guest_readiness_is_discarded() {
        use super::super::{
            orbit_auto_totem as totem, orbit_base_finder as base, orbit_fly as fly,
            orbit_freecam as freecam, orbit_no_fall as no_fall, orbit_spider as spider,
            orbit_step as step,
        };
        let cue = |name: &str, values: &[f32]| ModCue { name: name.into(), values: values.to_vec() };
        let requests = [
            cue(totem::REQUEST, &[1.0, 12.0]),
            cue(freecam::REQUEST, &[1.0, 10.0]),
            cue(fly::REQUEST, &[1.0, 11.0]),
            cue(no_fall::REQUEST, &[1.0]),
            cue(spider::REQUEST, &[1.0]),
            cue(step::REQUEST, &[1.0, 1.5]),
            cue(base::REQUEST, &[1.0, 5.0]),
        ];
        let mut denied = Merged::default();
        for request in &requests {
            assert!(denied.absorb_extended(request, 0, true));
        }
        assert!(denied.auto_totem.is_none() && denied.freecam.is_none() && denied.fly.is_none());
        assert!(!denied.no_fall && !denied.spider && denied.step.is_none());
        assert!(denied.base_finder.is_none());
        let mut merged = Merged::default();
        for request in &requests {
            assert!(merged.absorb_extended(request, u16::MAX, true));
        }
        assert_eq!(merged.auto_totem, Some(totem::Policy { health: 12.0 }));
        assert_eq!(merged.freecam, Some(freecam::Policy { speed: 10.0 }));
        assert_eq!(merged.fly, Some(fly::Policy { speed: 11.0 }));
        assert!(merged.no_fall && merged.spider);
        assert_eq!(merged.step, Some(step::Policy { height: 1.5 }));
        assert_eq!(merged.base_finder, Some(base::Policy { sensitivity: 5.0 }));
        assert!(merged.absorb_extended(&cue(fly::READY, &[1.0, 1.0]), u16::MAX, true));
        assert!(merged.absorb_extended(&cue(base::BASE, &[1.0, 0.0, 0.0, 0.0, 9.0, 1.0]), u16::MAX, true));
        assert!(!merged.absorb_extended(&cue("orbit.other", &[1.0]), u16::MAX, true));
        let mut malformed = Merged::default();
        malformed.absorb_extended(&cue(no_fall::REQUEST, &[2.0]), u16::MAX, true);
        malformed.absorb_extended(&cue(fly::REQUEST, &[1.0, 31.0]), u16::MAX, true);
        assert!(!malformed.no_fall && malformed.fly.is_none());
    }
    #[test]
    fn first_person_rig_applies_only_to_the_granted_rig_owner() {
        use super::super::orbit_camera_rig as rig;
        let request = ModCue { name: rig::REQUEST.into(), values: rig::values().to_vec() };
        let camera = ModGrants { camera: true, ..ModGrants::default() };
        assert_ne!(extended_capabilities(&camera) & rig::CAPABILITY, 0);
        assert_eq!(extended_capabilities(&ModGrants::default()) & rig::CAPABILITY, 0);
        let mut other = Merged::default();
        assert!(other.absorb_extended(&request, rig::CAPABILITY, false));
        assert!(!other.first_person_rig);
        let mut denied = Merged::default();
        assert!(denied.absorb_extended(&request, 0, true));
        assert!(!denied.first_person_rig);
        let mut malformed = Merged::default();
        let bad = ModCue { name: rig::REQUEST.into(), values: vec![1.0, 0.0] };
        assert!(malformed.absorb_extended(&bad, rig::CAPABILITY, true));
        assert!(!malformed.first_person_rig);
        let mut owner = Merged::default();
        assert!(owner.absorb_extended(&request, rig::CAPABILITY, true));
        assert!(owner.first_person_rig);
    }
    #[test]
    fn hitbox_support_requires_render_and_interaction_authority() {
        let granted = ModGrants {
            controls: true,
            player_state: true,
            players: true,
            entities: true,
            interaction: true,
            render: true,
            ..ModGrants::default()
        };
        let bit = super::super::orbit_hitbox::CAPABILITY;
        assert_ne!(assist_capabilities(&granted) & bit, 0);
        for field in 0..6 {
            let mut missing = granted.clone();
            match field {
                0 => missing.controls = false,
                1 => missing.player_state = false,
                2 => missing.players = false,
                3 => missing.entities = false,
                4 => missing.interaction = false,
                _ => missing.render = false,
            }
            assert_eq!(assist_capabilities(&missing) & bit, 0);
        }
    }
}
