//! Rollback-safe one-shot sound effects.
//!
//! The simulation declares a sound by spawning a [`RollbackAudioPlayer`]: an
//! *intent* naming the sound by asset path. It is ordinary rolled-back state,
//! and nothing about it depends on whether audio exists, so a headless run of
//! the simulation spawns and despawns exactly the same intents as a windowed
//! one. Playback is a separate, presentation-only plugin.
//!
//! - [`RollbackAudioPlugin`] (simulation): stamps each new intent with the
//!   rollback time it expires at, [`RollbackAudioPlugin::keep_for`] after it
//!   started, and despawns it once the clock reaches that. Add it wherever the
//!   simulation runs, headless included.
//! - [`RollbackAudioPlaybackPlugin`] (presentation): once per frame, plays
//!   each intent it has not played yet, keyed by `(path, expiry)`. It
//!   reads the simulation clock as `Time<C>`: a clock the game keeps in step
//!   with the rollback schedule's `Time` and rolls back.
//!
//! An intent lives only as long as a rollback could still take it back, which
//! is what `keep_for` is: at least the longest rollback the session can do
//! (the GGRS max prediction window), plus room for a slow frame so playback
//! sees every intent at least once. It has nothing to do with the length of
//! the clip, so no asset is ever read to decide it.
//!
//! When an intent disappears, its expiry says why:
//!
//! - still ahead of the rollback clock: a rollback undid it (or a seek went
//!   back before the sound), and the sound is cut;
//! - reached: it expired, and the sound plays out to its end.
//!
//! The intent carries its expiry rather than its start so that a world saved
//! mid-session can carry it: a save writes the time left, and a load that
//! restarts the clock reads it back as is.
//!
//! A resimulation that re-crosses the same edge spawns an intent with the same
//! key, which matches the sound already playing, so it is not played twice.
//!
//! Playback can hold a sound back: [`RollbackAudioDelays`] maps an asset path
//! to how long after its intent started the sound plays, on the same clock.
//! It is presentation, not simulation: how a sound lines up with what is on
//! screen, tuned without touching the rolled-back state. A delayed sound that
//! a rollback undoes before it plays never plays, and one whose delay outlasts
//! its intent's `keep_for` still plays once its time comes.

use std::{borrow::Cow, marker::PhantomData, time::Duration};

use bevy::{
    audio::PlaybackMode,
    platform::collections::{HashMap, HashSet},
    prelude::*,
};
#[cfg(feature = "bevy_ggrs")]
use bevy_ggrs::ReflectRollback;

use crate::{RollbackPostUpdate, RollbackPreUpdate};

/// Simulation half of rollback audio: expiries.
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
/// (expiry) and `RollbackPostUpdate` (stamping), for ordering
/// your own systems against.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RollbackAudioSystems;

/// Presentation half of rollback audio: plays the intents.
///
/// `Time<C>` is the simulation clock as of the last tick: the same elapsed
/// time the rollback schedule's `Time` had on it, and rolled back with the
/// world, so a seek or a rollback to an earlier tick takes it back too.
/// Playback compares expiries against it. Needs [`RollbackAudioPlugin`]
/// and Bevy's audio and asset plugins.
pub struct RollbackAudioPlaybackPlugin<C>(PhantomData<C>);

impl<C> Default for RollbackAudioPlaybackPlugin<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<C: Default + Send + Sync + 'static> Plugin for RollbackAudioPlaybackPlugin<C> {
    fn build(&self, app: &mut App) {
        app.init_resource::<RollbackAudioDelays>();
        app.add_systems(Update, sync_rollback_sounds::<C>);
    }
}

/// How long after its intent started each sound plays, by asset path; a path
/// not listed plays at once. Presentation only: see the module docs.
#[derive(Resource, Clone, Debug, Default)]
pub struct RollbackAudioDelays(pub HashMap<Cow<'static, str>, Duration>);

impl RollbackAudioDelays {
    /// The delay for `path`, zero when none is set.
    pub fn get(&self, path: &str) -> Duration {
        self.0.get(path).copied().unwrap_or_default()
    }
}

/// When the sound for an intent plays: its start (`keep_for` before its
/// expiry) plus its delay.
fn play_at(expiry: Duration, keep_for: Duration, delay: Duration) -> Duration {
    expiry.saturating_sub(keep_for) + delay
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

/// The rollback time an intent expires at: `keep_for` after it started. Added
/// by [`start_rollback_sounds`] in `RollbackPostUpdate` of the tick it was
/// spawned on.
#[derive(Component, Clone, Copy, Debug, Reflect)]
#[cfg_attr(feature = "bevy_ggrs", derive(bevy_ggrs::Rollback))]
#[cfg_attr(feature = "bevy_ggrs", rollback(copy))]
#[cfg_attr(feature = "bevy_ggrs", reflect(Component, Rollback))]
#[cfg_attr(not(feature = "bevy_ggrs"), reflect(Component))]
pub struct RollbackAudioPlayerExpiry(pub Duration);

/// [`RollbackAudioPlugin::keep_for`], for playback to tell an undone intent
/// from an expired one.
#[derive(Resource, Clone, Copy, Debug)]
pub struct RollbackAudioKeepFor(pub Duration);

/// A sound that is playing for an intent. Despawned by Bevy when it ends.
#[derive(Component)]
pub struct RollbackAudioPlayerInstance {
    /// The rollback time its intent expires at.
    pub expiry: Duration,
}

/// Stamp each new intent with the rollback time it expires at.
pub fn start_rollback_sounds(
    mut commands: Commands,
    new: Query<
        Entity,
        (
            With<RollbackAudioPlayer>,
            Without<RollbackAudioPlayerExpiry>,
        ),
    >,
    keep_for: Res<RollbackAudioKeepFor>,
    time: Res<Time>,
) {
    let expiry = time.elapsed() + keep_for.0;
    for entity in &new {
        commands
            .entity(entity)
            .insert(RollbackAudioPlayerExpiry(expiry));
    }
}

/// Despawn each intent whose expiry the clock has reached.
pub fn expire_rollback_sounds(
    mut commands: Commands,
    intents: Query<(Entity, &RollbackAudioPlayerExpiry)>,
    time: Res<Time>,
) {
    for (entity, expiry) in &intents {
        if expiry.0 <= time.elapsed() {
            commands.entity(entity).despawn();
        }
    }
}

/// Play each intent not played yet once its delay is up, and cut the sound of
/// one a rollback undid.
pub fn sync_rollback_sounds<C: Default + Send + Sync + 'static>(
    mut commands: Commands,
    intents: Query<(&RollbackAudioPlayer, &RollbackAudioPlayerExpiry)>,
    instances: Query<(), With<RollbackAudioPlayerInstance>>,
    now: Res<Time<C>>,
    keep_for: Res<RollbackAudioKeepFor>,
    delays: Res<RollbackAudioDelays>,
    asset_server: Res<AssetServer>,
    // `None` while the sound waits out its delay.
    mut played: Local<HashMap<(Cow<'static, str>, Duration), Option<Entity>>>,
) {
    let desired: HashSet<(Cow<'static, str>, Duration)> = intents
        .iter()
        .map(|(player, expiry)| (player.0.clone(), expiry.0))
        .collect();
    for key in &desired {
        played.entry(key.clone()).or_insert(None);
    }

    let now = now.elapsed();
    played.retain(|key, instance| {
        let live = desired.contains(key);
        // Gone with its expiry still ahead: a rollback or a seek undid it.
        // Cut the sound, or never play it.
        if !live && key.1 > now {
            if let Some(entity) = instance.filter(|e| instances.contains(*e)) {
                commands.entity(entity).despawn();
            }
            return false;
        }
        if instance.is_none() && now >= play_at(key.1, keep_for.0, delays.get(&key.0)) {
            *instance = Some(
                commands
                    .spawn((
                        AudioPlayer::<AudioSource>(asset_server.load(key.0.to_string())),
                        PlaybackSettings {
                            mode: PlaybackMode::Despawn,
                            ..default()
                        },
                        RollbackAudioPlayerInstance { expiry: key.1 },
                    ))
                    .id(),
            );
        }
        // Kept while its intent lives, and past that while it still waits to
        // play; an expired one that played is let play out.
        live || instance.is_none()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sound_plays_its_delay_after_its_intent_started() {
        let ms = Duration::from_millis;
        // Started at 1000 ms, kept for 250, so it expires at 1250.
        assert_eq!(play_at(ms(1250), ms(250), Duration::ZERO), ms(1000));
        assert_eq!(play_at(ms(1250), ms(250), ms(150)), ms(1150));
        // A delay past keep_for plays after the intent is gone.
        assert_eq!(play_at(ms(1250), ms(250), ms(400)), ms(1400));
    }
}
