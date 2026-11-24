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
//!   each intent it has not played yet, keyed by its path, its expiry and
//!   its key, if it has one. It reads the simulation clock as `Time<C>`: a
//!   clock the game keeps in step with the rollback schedule's `Time` and
//!   rolls back.
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
//! The key is what the intent holds, never its entity: a rollback takes the
//! entity away and the resimulation spawns another.
//!
//! An intent is a path and nothing more, so two for one path on one tick
//! are one sound. A game says more with two components of its own, spawned
//! beside the [`RollbackAudioPlayer`] and named as the playback plugin's
//! types ([`RollbackAudioPlaybackPlugin::new`]):
//!
//! - A **key** `K`: what tells two sounds of one path on one tick apart,
//!   such as who made each. Two intents whose keys differ are two sounds.
//!   Pick something a resimulation gives back the same, an index and never
//!   a float out of the simulation: a key that comes back changed is a new
//!   sound, and the one playing is cut. An intent with no key shares its
//!   sound with every other of its path and tick that has none, which is a
//!   choice too: one knock a tick however many things hit.
//! - **Data** `D`: what the game wants to know of the sound when it plays,
//!   such as where it is. It tells no two sounds apart. When a
//!   resimulation declares the same sound with other data, the sound plays
//!   on and takes the new data.
//!
//! Playback puts a clone of each on the sound it starts.
//!
//! Playback can hold a sound back: [`RollbackAudioDelays`] maps an asset path
//! to how long after its intent started the sound plays, on the same clock.
//! It is presentation, not simulation: how a sound lines up with what is on
//! screen, tuned without touching the rolled-back state. A delayed sound that
//! a rollback undoes before it plays never plays, and one whose delay outlasts
//! its intent's `keep_for` still plays once its time comes.

use std::{borrow::Cow, marker::PhantomData, time::Duration};

use bevy::{audio::PlaybackMode, platform::collections::HashMap, prelude::*};
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
///
/// `K` and `D` are the key and the data an intent may carry (see the
/// module docs): components spawned beside its [`RollbackAudioPlayer`] and
/// cloned onto the sound that plays for it. The key tells two sounds of one
/// path on one tick apart; the data does not. `default()` reads neither;
/// [`new`](Self::new) names them.
pub struct RollbackAudioPlaybackPlugin<C, K = Nothing, D = Nothing>(PhantomData<(C, K, D)>);

/// The key, or the data, of a playback plugin that reads none: `()` is no
/// component. Nothing spawns it.
#[derive(Component, Clone, PartialEq, Debug)]
pub struct Nothing;

// For `Nothing` alone, so `RollbackAudioPlaybackPlugin::<C>::default()`
// names its type in full: a default type is not inferred in an expression.
impl<C> Default for RollbackAudioPlaybackPlugin<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<C, K, D> RollbackAudioPlaybackPlugin<C, K, D> {
    /// Playback that reads the key `K` and the data `D` off each intent
    /// that carries them. [`Nothing`] for one a game has no use for.
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<C, K, D> Plugin for RollbackAudioPlaybackPlugin<C, K, D>
where
    C: Default + Send + Sync + 'static,
    K: Component + Clone + PartialEq,
    D: Component + Clone + PartialEq,
{
    fn build(&self, app: &mut App) {
        app.init_resource::<RollbackAudioDelays>();
        app.add_systems(Update, sync_rollback_sounds::<C, K, D>);
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
#[cfg_attr(feature = "bevy_ggrs", component(summary_tick))]
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
#[cfg_attr(feature = "bevy_ggrs", component(summary_tick))]
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

/// A sound playback knows of, by what its intent held.
pub struct PlayedSound<K, D> {
    path: Cow<'static, str>,
    expiry: Duration,
    key: Option<K>,
    data: Option<D>,
    /// `None` while the sound waits out its delay.
    instance: Option<Entity>,
}

impl<K: PartialEq, D> PlayedSound<K, D> {
    fn is_for(&self, path: &str, expiry: Duration, key: Option<&K>) -> bool {
        self.expiry == expiry && self.path == path && self.key.as_ref() == key
    }
}

/// Play each intent not played yet once its delay is up, and cut the sound of
/// one a rollback undid.
pub fn sync_rollback_sounds<C, K, D>(
    mut commands: Commands,
    intents: Query<(
        &RollbackAudioPlayer,
        &RollbackAudioPlayerExpiry,
        Option<&K>,
        Option<&D>,
    )>,
    instances: Query<(), With<RollbackAudioPlayerInstance>>,
    now: Res<Time<C>>,
    keep_for: Res<RollbackAudioKeepFor>,
    delays: Res<RollbackAudioDelays>,
    asset_server: Res<AssetServer>,
    mut played: Local<Vec<PlayedSound<K, D>>>,
) where
    C: Default + Send + Sync + 'static,
    K: Component + Clone + PartialEq,
    D: Component + Clone + PartialEq,
{
    for (player, expiry, key, data) in &intents {
        if !played.iter().any(|p| p.is_for(&player.0, expiry.0, key)) {
            played.push(PlayedSound {
                path: player.0.clone(),
                expiry: expiry.0,
                key: key.cloned(),
                data: data.cloned(),
                instance: None,
            });
        }
    }

    let now = now.elapsed();
    played.retain_mut(|sound| {
        // The data of each intent this sound is for.
        let mut live = intents
            .iter()
            .filter(|(player, expiry, key, _)| sound.is_for(&player.0, expiry.0, *key))
            .map(|(.., data)| data)
            .peekable();
        let is_live = live.peek().is_some();
        // Gone with its expiry still ahead: a rollback or a seek undid it.
        // Cut the sound, or never play it.
        if !is_live && sound.expiry > now {
            if let Some(entity) = sound.instance.filter(|e| instances.contains(*e)) {
                commands.entity(entity).despawn();
            }
            return false;
        }
        // None of its intents holds the data it has: a resimulation
        // declared it anew with other data. It plays on and takes the new.
        // While one of them still does it keeps what it has, so a sound
        // several intents share does not hop between theirs.
        if let Some(&first) = live.peek() {
            if !live.any(|data| data == sound.data.as_ref()) {
                sound.data = first.cloned();
                if let (Some(entity), Some(data)) = (
                    sound.instance.filter(|e| instances.contains(*e)),
                    &sound.data,
                ) {
                    commands.entity(entity).insert(data.clone());
                }
            }
        }
        if sound.instance.is_none()
            && now >= play_at(sound.expiry, keep_for.0, delays.get(&sound.path))
        {
            let playing = (
                AudioPlayer::<AudioSource>(asset_server.load(sound.path.to_string())),
                PlaybackSettings {
                    mode: PlaybackMode::Despawn,
                    ..default()
                },
                RollbackAudioPlayerInstance {
                    expiry: sound.expiry,
                },
            );
            // In one spawn, so whatever watches for the sound finds its key
            // and its data on it.
            sound.instance = Some(match (&sound.key, &sound.data) {
                (Some(key), Some(data)) => {
                    commands.spawn((playing, key.clone(), data.clone())).id()
                }
                (Some(key), None) => commands.spawn((playing, key.clone())).id(),
                (None, Some(data)) => commands.spawn((playing, data.clone())).id(),
                (None, None) => commands.spawn(playing).id(),
            });
        }
        // Kept while its intent lives, and past that while it still waits to
        // play; an expired one that played is let play out.
        is_live || sound.instance.is_none()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Who made a sound: a game's key.
    #[derive(Component, Clone, PartialEq, Debug)]
    struct Maker(u32);

    /// Where a sound is: a game's data.
    #[derive(Component, Clone, PartialEq, Debug)]
    struct Place(i32);

    const KEEP_FOR: Duration = Duration::from_millis(250);

    /// Playback that reads [`Maker`] and [`Place`], with the clock at one
    /// second and no audio device: a sound is the entity playback spawns
    /// for it.
    fn playback() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()));
        app.init_asset::<AudioSource>();
        app.insert_resource(RollbackAudioKeepFor(KEEP_FOR));
        app.add_plugins(RollbackAudioPlaybackPlugin::<Fixed, Maker, Place>::new());
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .advance_by(Duration::from_secs(1));
        app
    }

    /// An intent for `jump.ogg` started now, by `maker` at `place`, when
    /// given.
    fn declare(app: &mut App, maker: Option<u32>, place: Option<i32>) -> Entity {
        let expiry = app.world().resource::<Time<Fixed>>().elapsed() + KEEP_FOR;
        let mut intent = app.world_mut().spawn((
            RollbackAudioPlayer::new("jump.ogg"),
            RollbackAudioPlayerExpiry(expiry),
        ));
        if let Some(maker) = maker {
            intent.insert(Maker(maker));
        }
        if let Some(place) = place {
            intent.insert(Place(place));
        }
        intent.id()
    }

    /// Each sound playing, with the key and the data on it, sorted.
    fn sounds(app: &mut App) -> Vec<(Entity, Option<u32>, Option<i32>)> {
        let world = app.world_mut();
        let mut sounds: Vec<_> = world
            .query_filtered::<(Entity, Option<&Maker>, Option<&Place>), With<RollbackAudioPlayerInstance>>()
            .iter(world)
            .map(|(sound, maker, place)| (sound, maker.map(|m| m.0), place.map(|p| p.0)))
            .collect();
        sounds.sort_by_key(|&(_, maker, place)| (maker, place));
        sounds
    }

    /// The key and the data on each sound playing, sorted.
    fn heard(app: &mut App) -> Vec<(Option<u32>, Option<i32>)> {
        sounds(app)
            .into_iter()
            .map(|(_, maker, place)| (maker, place))
            .collect()
    }

    #[test]
    fn a_sound_carries_its_intents_key_and_data() {
        let mut app = playback();
        declare(&mut app, Some(3), Some(7));
        app.update();
        assert_eq!(heard(&mut app), [(Some(3), Some(7))]);
    }

    #[test]
    fn an_intent_with_neither_plays_as_before() {
        let mut app = playback();
        declare(&mut app, None, None);
        app.update();
        assert_eq!(heard(&mut app), [(None, None)]);
    }

    #[test]
    fn two_intents_on_one_tick_are_two_sounds_when_their_keys_differ() {
        let mut app = playback();
        declare(&mut app, Some(1), Some(10));
        declare(&mut app, Some(2), Some(20));
        app.update();
        assert_eq!(heard(&mut app), [(Some(1), Some(10)), (Some(2), Some(20))]);
    }

    /// Data tells no two sounds apart: with no key, or one key, two intents
    /// are one sound wherever each says it is.
    #[test]
    fn two_intents_with_one_key_or_none_are_one_sound_whatever_their_data() {
        for maker in [None, Some(1)] {
            let mut app = playback();
            declare(&mut app, maker, Some(10));
            declare(&mut app, maker, Some(20));
            app.update();
            let playing = heard(&mut app);
            assert_eq!(playing.len(), 1, "{maker:?}: {playing:?}");
            // It keeps the data it took, on every frame after.
            for _ in 0..3 {
                app.update();
                assert_eq!(heard(&mut app), playing, "{maker:?}");
            }
        }
    }

    /// A rollback takes the intent away and the resimulation spawns it
    /// again, a new entity with the same key: the sound plays on, once.
    #[test]
    fn a_resimulated_intent_matches_the_sound_already_playing() {
        let mut app = playback();
        let intent = declare(&mut app, Some(1), Some(10));
        app.update();
        let first = sounds(&mut app);
        app.world_mut().despawn(intent);
        declare(&mut app, Some(1), Some(10));
        app.update();
        assert_eq!(sounds(&mut app), first);
    }

    /// The resimulation puts the same sound somewhere else: it is the same
    /// sound, playing on, with the new data.
    #[test]
    fn a_resimulated_intent_with_other_data_moves_the_sound_and_does_not_restart_it() {
        let mut app = playback();
        let intent = declare(&mut app, Some(1), Some(10));
        app.update();
        let first = sounds(&mut app)[0].0;
        app.world_mut().despawn(intent);
        declare(&mut app, Some(1), Some(12));
        app.update();
        app.update();
        assert_eq!(sounds(&mut app), [(first, Some(1), Some(12))]);
    }

    /// A rollback that undoes one of two intents cuts its sound and leaves
    /// the other's.
    #[test]
    fn an_undone_intent_cuts_its_own_sound_and_no_other() {
        let mut app = playback();
        let undone = declare(&mut app, Some(1), Some(10));
        declare(&mut app, Some(2), Some(20));
        app.update();
        app.world_mut().despawn(undone);
        app.update();
        assert_eq!(heard(&mut app), [(Some(2), Some(20))]);
    }

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
