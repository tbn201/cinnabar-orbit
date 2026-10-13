use bevy::{prelude::Resource, render::extract_resource::ExtractResource};
use mod_api::MAX_BLOCK_HIGHLIGHTS;
use mod_render::{RenderOutput, block_highlights, geometry::ModVertex, position_box};
use std::sync::Arc;
/// The current mod's render output, extracted whenever the mod commits a change.
#[derive(Resource, Clone, Debug, Default)]
pub struct ModRenderScene {
    generation: u64,
    pub passes: Vec<mod_render::Pass>,
    pub vertices: Arc<[ModVertex]>,
    primitives: Arc<mod_render::Primitives>,
    pub marker_vertices: Arc<[ModVertex]>,
    position_box: Option<[[f32; 3]; 2]>,
    hitboxes: Vec<[[f32; 3]; 2]>,
    pub block_vertices: Arc<[ModVertex]>,
    block_positions: Arc<[[i32; 3]]>,
    block_color: [f32; 4],
}

impl ExtractResource<bevy::render::RenderApp> for ModRenderScene {
    type Source = Self;

    fn extract_resource(source: &Self) -> Self {
        source.clone()
    }
}

impl ModRenderScene {
    /// Adopts `output` unless `generation` is already applied.
    pub fn apply(&mut self, output: &RenderOutput, generation: u64) {
        if generation == self.generation {
            return;
        }
        self.generation = generation;
        self.passes.clone_from(&output.passes);
        if !Arc::ptr_eq(&self.primitives, &output.primitives) {
            self.primitives = Arc::clone(&output.primitives);
            self.vertices = mod_render::geometry::build(&output.primitives).into();
        }
    }

    /// Publishes a host-owned own-position box independently of guest render output.
    pub fn set_position_box(&mut self, bounds: Option<[[f32; 3]; 2]>) {
        let bounds = bounds.filter(position_box::valid);
        if self.position_box == bounds {
            return;
        }
        self.position_box = bounds;
        self.rebuild_markers();
    }

    /// Highlights loaded unit blocks through world geometry, independently of guest primitives.
    pub fn set_block_highlights(&mut self, positions: &[[i32; 3]], color: [f32; 4]) {
        let positions = &positions[..positions.len().min(MAX_BLOCK_HIGHLIGHTS)];
        let valid = color
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v));
        let positions = if valid { positions } else { &[] };
        if self.block_positions.as_ref() == positions && self.block_color == color {
            return;
        }
        self.block_positions = Arc::from(positions);
        self.block_color = color;
        self.block_vertices = block_highlights::build(positions, color).into();
    }

    /// Drops every pass and primitive, as when a mod traps, reloads or is revoked.
    pub fn clear(&mut self) {
        if self.generation != 0 || !self.passes.is_empty() || self.vertex_count() != 0 {
            *self = Self::default();
        }
    }

    pub fn set_hitboxes(&mut self, bounds: &[[[f32; 3]; 2]]) {
        let bounds: Vec<_> = bounds
            .iter()
            .take(mod_api::MAX_OBSERVED_ACTORS * mod_api::MAX_OBSERVED_ACTOR_BOXES)
            .filter(|bounds| {
                position_box::valid(bounds)
                    && bounds
                        .iter()
                        .flatten()
                        .all(|value| value.abs() <= mod_api::MAX_PRIMITIVE_COORDINATE)
            })
            .copied()
            .collect();
        if self.hitboxes == bounds {
            return;
        }
        self.hitboxes = bounds;
        self.rebuild_markers();
    }

    fn rebuild_markers(&mut self) {
        let mut vertices = Vec::new();
        if let Some(bounds) = self.position_box {
            position_box::append(&mut vertices, bounds);
        }
        for bounds in &self.hitboxes {
            position_box::append_outline(&mut vertices, *bounds);
        }
        self.marker_vertices = vertices.into();
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len() + self.marker_vertices.len() + self.block_vertices.len()
    }
}

#[cfg(test)]
mod block_highlights_tests;
#[cfg(test)]
mod position_box_tests;

#[cfg(test)]
mod hitbox_tests {
    use super::*;
    #[test]
    fn outlines_keep_the_local_marker_and_clear_without_leaving_geometry() {
        let mut scene = ModRenderScene::default();
        let bounds = [[0.0; 3], [1.0; 3]];
        scene.set_position_box(Some(bounds));
        let local = scene.marker_vertices.len();
        scene.set_hitboxes(&[bounds]);
        assert_eq!(
            scene.marker_vertices.len(),
            local + position_box::OUTLINE_VERTICES
        );
        let cached = Arc::clone(&scene.marker_vertices);
        scene.set_hitboxes(&[bounds]);
        assert!(Arc::ptr_eq(&cached, &scene.marker_vertices));
        scene.set_position_box(None);
        assert_eq!(scene.marker_vertices.len(), position_box::OUTLINE_VERTICES);
        scene.set_hitboxes(&[]);
        assert!(scene.marker_vertices.is_empty());
    }
    #[test]
    fn invalid_boxes_are_rejected_and_geometry_has_a_fixed_budget() {
        let mut scene = ModRenderScene::default();
        scene.set_hitboxes(&[[[f32::NAN; 3], [1.0; 3]], [[1.0; 3], [0.0; 3]]]);
        assert!(scene.marker_vertices.is_empty());
        let limit = mod_api::MAX_OBSERVED_ACTORS * mod_api::MAX_OBSERVED_ACTOR_BOXES;
        scene.set_hitboxes(&vec![[[0.0; 3], [1.0; 3]]; limit + 1]);
        assert_eq!(
            scene.marker_vertices.len(),
            limit * position_box::OUTLINE_VERTICES
        );
        scene.clear();
        assert!(scene.marker_vertices.is_empty());
    }
}
