use bevy::prelude::*;

/// Replacement for Bevy's FrameCount, not tied to rendering
///
/// Keeps track of the current rollback frame
///
/// Note: you need to manually add `increase_frame_count` to the rollback schedule
/// for this resource to be updated.
#[derive(Resource, Default, Reflect, Hash, Clone, Copy, Debug)]
#[reflect(Hash)]
pub struct RollFrameCount(pub u32);

pub fn increase_frame_count(mut frame_count: ResMut<RollFrameCount>) {
    frame_count.0 = frame_count.0.wrapping_add(1);
}
