//! Rollback-safe one-shot sound effects.
//!
//! The simulation declares a sound by spawning a [`RollbackAudioPlayer`]: an
//! *intent* naming the sound by asset path. It is ordinary rolled-back state,
//! and nothing about it depends on whether audio exists, so a headless run of
//! the simulation spawns and despawns exactly the same intents as a windowed
//! one. Playback is a separate, presentation-only plugin.
//!
//! - [`RollbackAudioPlugin`] (simulation): stamps each new intent with the
//!   rollback time it started at, and despawns it once it is older than
//!   [`RollbackAudioPlugin::keep_for`]. Add it wherever the simulation runs,
//!   headless included.
//! - [`RollbackAudioPlaybackPlugin`] (presentation): once per frame, plays
//!   each intent it has not played yet, keyed by `(path, start time)`. It
//!   reads the simulation clock as `Time<C>`: a clock the game keeps in step
//!   with the rollback schedule's `Time` and rolls back.
//!
//! An intent lives only as long as a rollback could still take it back, which
//! is what `keep_for` is: at least the longest rollback the session can do
//! (the GGRS max prediction window), plus room for a slow frame so playback
//! sees every intent at least once. It has nothing to do with the length of
//! the clip, so no asset is ever read to decide it.
//!
//! When an intent disappears, its start time says why:
//!
//! - started less than `keep_for` before the rollback clock, or after it (a
//!   seek back before the sound): a rollback undid it, and the sound is cut;
//! - started earlier: it expired, and the sound plays out to its end.
//!
//! A resimulation that re-crosses the same edge spawns an intent with the same
//! key, which matches the sound already playing, so it is not played twice.

use std::{borrow::Cow, marker::PhantomData, time::Duration};

use bevy::{
    audio::PlaybackMode,
    platform::collections::{HashMap, HashSet},
    prelude::*,
};
#[cfg(feature = "bevy_ggrs")]
use bevy_ggrs::ReflectRollback;

use crate::{RollbackPostUpdate, RollbackPreUpdate};

/// Simulation half of rollback audio: start times and expiry.
///
/// ```rust
/// # use bevy::prelude::*;
/// # use bevy_roll_safe::prelude::*;
/// # fn start() {
/// fn main() {
/// # let mut app = App::new();
///     app.add_plugins((
///         RollbackSchedulePlugin::new(FixedUpdate),
///         RollbackAudioPlugin::default(),
///         // Only where sound is played, reading your rolled-back sim clock.
///         RollbackAudioPlaybackPlugin::<Fixed>::default(),
///     ));
/// }
/// # }
/// fn on_game_over(mut commands: Commands) {
///     commands.spawn(RollbackAudioPlayer::new("sounds/game_over.ogg"));
/// }
/// ```
pub struct RollbackAudioPlugin {
    /// How long an intent lives after it starts, in rollback time. Must cover
    /// the longest rollback the session can do; see the module docs.
    pub keep_for: Duration,
}

impl Default for RollbackAudioPlugin {
    fn default() -> Self {
        Self {
            keep_for: Duration::from_millis(250),
        }
    }
}

impl Plugin for RollbackAudioPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RollbackAudioKeepFor(self.keep_for));
        app.add_systems(
            RollbackPreUpdate,
            expire_rollback_sounds.in_set(RollbackAudioSystems),
        );
        app.add_systems(
            RollbackPostUpdate,
            start_rollback_sounds.in_set(RollbackAudioSystems),
        );
    }
}

/// The simulation systems of [`RollbackAudioPlugin`], in `RollbackPreUpdate`
/// (expiry) and `RollbackPostUpdate` (start times), for ordering
/// your own systems against.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RollbackAudioSystems;

/// Presentation half of rollback audio: plays the intents.
///
/// `Time<C>` is the simulation clock as of the last tick: the same elapsed
/// time the rollback schedule's `Time` had on it, and rolled back with the
/// world, so a seek or a rollback to an earlier tick takes it back too.
/// Playback compares start times against it. Needs [`RollbackAudioPlugin`]
/// and Bevy's audio and asset plugins.
pub struct RollbackAudioPlaybackPlugin<C>(PhantomData<C>);

impl<C> Default for RollbackAudioPlaybackPlugin<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<C: Default + Send + Sync + 'static> Plugin for RollbackAudioPlaybackPlugin<C> {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, sync_rollback_sounds::<C>);
    }
}

/// A one-shot sound the simulation declares, named by asset path.
///
/// Spawn it from a rollback schedule on the tick the sound should start. It is
/// write-only intent: playback reads it, nothing in the simulation should.
#[derive(Component, Clone, Debug, Reflect)]
#[cfg_attr(feature = "bevy_ggrs", derive(bevy_ggrs::Rollback))]
#[cfg_attr(feature = "bevy_ggrs", rollback(clone, marker))]
#[cfg_attr(feature = "bevy_ggrs", reflect(Component, Rollback))]
#[cfg_attr(not(feature = "bevy_ggrs"), reflect(Component))]
pub struct RollbackAudioPlayer(pub Cow<'static, str>);

impl RollbackAudioPlayer {
    pub fn new(path: impl Into<Cow<'static, str>>) -> Self {
        Self(path.into())
    }
}

/// The rollback time an intent started at. Added by [`start_rollback_sounds`]
/// in `RollbackPostUpdate` of the tick it was spawned on.
#[derive(Component, Clone, Copy, Debug, Reflect)]
#[cfg_attr(feature = "bevy_ggrs", derive(bevy_ggrs::Rollback))]
#[cfg_attr(feature = "bevy_ggrs", rollback(copy))]
#[cfg_attr(feature = "bevy_ggrs", reflect(Component, Rollback))]
#[cfg_attr(not(feature = "bevy_ggrs"), reflect(Component))]
pub struct RollbackAudioPlayerStartTime(pub Duration);

/// [`RollbackAudioPlugin::keep_for`], for playback to tell an undone intent
/// from an expired one.
#[derive(Resource, Clone, Copy, Debug)]
pub struct RollbackAudioKeepFor(pub Duration);

/// A sound that is playing for an intent. Despawned by Bevy when it ends.
#[derive(Component)]
pub struct RollbackAudioPlayerInstance {
    /// The rollback time its intent started at.
    pub start_time: Duration,
}

/// Stamp each new intent with the rollback time it starts at.
pub fn start_rollback_sounds(
    mut commands: Commands,
    new: Query<
        Entity,
        (
            With<RollbackAudioPlayer>,
            Without<RollbackAudioPlayerStartTime>,
        ),
    >,
    time: Res<Time>,
) {
    let start_time = time.elapsed();
    for entity in &new {
        commands
            .entity(entity)
            .insert(RollbackAudioPlayerStartTime(start_time));
    }
}

/// Despawn each intent `keep_for` after it started.
pub fn expire_rollback_sounds(
    mut commands: Commands,
    intents: Query<(Entity, &RollbackAudioPlayerStartTime)>,
    keep_for: Res<RollbackAudioKeepFor>,
    time: Res<Time>,
) {
    for (entity, start) in &intents {
        if start.0 + keep_for.0 <= time.elapsed() {
            commands.entity(entity).despawn();
        }
    }
}

/// Play each intent not played yet, and cut the sound of one a rollback undid.
pub fn sync_rollback_sounds<C: Default + Send + Sync + 'static>(
    mut commands: Commands,
    intents: Query<(&RollbackAudioPlayer, &RollbackAudioPlayerStartTime)>,
    instances: Query<(), With<RollbackAudioPlayerInstance>>,
    now: Res<Time<C>>,
    keep_for: Res<RollbackAudioKeepFor>,
    asset_server: Res<AssetServer>,
    mut played: Local<HashMap<(Cow<'static, str>, Duration), Entity>>,
) {
    let desired: HashSet<(Cow<'static, str>, Duration)> = intents
        .iter()
        .map(|(player, start)| (player.0.clone(), start.0))
        .collect();

    // Forget intents that are gone: cut the sound when a rollback or a seek
    // undid it, let it play out when it expired.
    let undone_after = now.elapsed().saturating_sub(keep_for.0);
    played.retain(|key, instance| {
        if desired.contains(key) {
            return true;
        }
        if key.1 > undone_after && instances.contains(*instance) {
            commands.entity(*instance).despawn();
        }
        false
    });

    for key in desired {
        if played.contains_key(&key) {
            continue;
        }
        let instance = commands
            .spawn((
                AudioPlayer::<AudioSource>(asset_server.load(key.0.to_string())),
                PlaybackSettings {
                    mode: PlaybackMode::Despawn,
                    ..default()
                },
                RollbackAudioPlayerInstance { start_time: key.1 },
            ))
            .id();
        played.insert(key, instance);
    }
}
