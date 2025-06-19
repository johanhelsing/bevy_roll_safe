use bevy::{
    audio::PlaybackMode,
    platform::collections::{HashMap, HashSet},
    prelude::*,
};
#[cfg(feature = "bevy_ggrs")]
use bevy_ggrs::RollbackApp;
use std::time::Duration;

use crate::{RollbackPostUpdate, RollbackPreUpdate};

/// Plugin for managing rollback audio effects in a Bevy application.
///
/// ```rust
/// # use bevy::prelude::*;
/// # use bevy_roll_safe::prelude::*;
/// # fn start() {
/// fn main() {
/// # let mut app = App::new();
///     app.add_plugins((RollbackSchedulePlugin::new(FixedUpdate), RollbackAudioPlugin));
/// }
///
/// # }
/// # #[derive(Resource)]
/// # struct Sounds {
/// #     game_over: Handle<AudioSource>,
/// # }
/// fn on_game_over(mut commands: Commands, sounds: Res<Sounds>) {
///     // Play a sound effect when the game is over
///     commands.spawn(RollbackAudioPlayer(
///          AudioPlayer::new(sounds.game_over.clone())
///     ));
/// }
///
/// ```
///
/// See [`RollbackAudioPlayer`] for more details on how to use this plugin.
pub struct RollbackAudioPlugin;

impl Plugin for RollbackAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, sync_rollback_sounds);
        app.add_systems(RollbackPreUpdate, remove_finished_sounds);
        app.add_systems(RollbackPostUpdate, start_rollback_sounds);

        #[cfg(feature = "bevy_ggrs")]
        {
            app.rollback_component_with_clone::<RollbackAudioPlayer>();
            app.rollback_component_with_clone::<RollbackAudioPlayerStartTime>();
            app.rollback_component_with_clone::<PlaybackSettings>();
            app.add_systems(RollbackPostUpdate, add_rollback_to_rollback_sounds);
        }
    }
}

/// Rollback-safe wrapper around [`AudioPlayer`].
///
/// Usage is almost identical to [`AudioPlayer`], but sounds will not be played
/// directly, instead another non-rollback entity will be spawned with the
/// actual audio player.
///
/// State will be synced once per frame, so if the sound effect is despawned
/// and respawned via rollback, the sound will continue playing without
/// interruption.
#[derive(Component, Clone)]
pub struct RollbackAudioPlayer(pub AudioPlayer);

impl From<AudioPlayer> for RollbackAudioPlayer {
    fn from(audio_player: AudioPlayer) -> Self {
        Self(audio_player)
    }
}

/// When the sound effect should have started playing
#[derive(Component, Clone, Debug)]
pub struct RollbackAudioPlayerStartTime(pub Duration);

/// Represents an instance of a rollback sound effect that is currently playing
#[derive(Component)]
pub struct RollbackAudioPlayerInstance {
    /// The desired start time in the rollback world's time
    desired_start_time: Duration,
}

#[derive(PartialEq, Eq, Hash)]
struct PlayingRollbackAudioKey {
    audio_source: Handle<AudioSource>,
    start_time: Duration,
    // TODO: add more keys as appropriate if sound effects are colliding
}

/// Updates playing sounds to match the desired state
/// spawns any missing sounds that should be playing.
/// and despawns any sounds that should not be playing.
pub fn sync_rollback_sounds(
    mut commands: Commands,
    rollback_audio_players: Query<(
        &RollbackAudioPlayer,
        &RollbackAudioPlayerStartTime,
        Option<&PlaybackSettings>,
    )>,
    instances: Query<(Entity, &RollbackAudioPlayerInstance, &AudioPlayer)>,
) {
    // todo: Ideally we would use a HashSet with settings, but PlaybackSettings
    // is not hashable. So we use a HashMap with the key being the audio source
    // and start time. This likely leads to some collisions, but leaving as is
    // for now.
    let desired_state: HashMap<PlayingRollbackAudioKey, Option<&PlaybackSettings>> =
        rollback_audio_players
            .iter()
            .map(|(player, start_time, playback_settings)| {
                (
                    PlayingRollbackAudioKey {
                        audio_source: player.0 .0.clone(),
                        start_time: start_time.0,
                    },
                    playback_settings,
                )
            })
            .collect();

    let mut playing_sounds = HashSet::new();

    for (instance_entity, instance, audio_player) in &instances {
        let rollback_sound_key = PlayingRollbackAudioKey {
            audio_source: audio_player.0.clone(),
            start_time: instance.desired_start_time,
        };

        // if the playing sound is not in the desired state, despawn it
        if !desired_state.contains_key(&rollback_sound_key) {
            commands.entity(instance_entity).despawn();
        } else {
            playing_sounds.insert(rollback_sound_key);
        }
    }

    // spawn any missing sounds
    for (sound, settings) in desired_state {
        if playing_sounds.contains(&sound) {
            // if the sound is already playing, skip it
            continue;
        }

        debug!("Spawning sound: {:?}", sound.audio_source);

        let settings = settings.cloned().unwrap_or(PlaybackSettings::ONCE);

        commands.spawn((
            AudioPlayer::new(sound.audio_source.clone()),
            settings,
            RollbackAudioPlayerInstance {
                desired_start_time: sound.start_time,
            },
        ));
    }
}

/// Starts the rollback sounds by recording the current time as the start time
pub fn start_rollback_sounds(
    mut commands: Commands,
    mut rollback_audio_players: Query<
        Entity,
        (
            With<RollbackAudioPlayer>,
            Without<RollbackAudioPlayerStartTime>,
        ),
    >,
    time: Res<Time>,
) {
    let start_time = time.elapsed();
    for entity in rollback_audio_players.iter_mut() {
        trace!("adding RollbackAudioPlayerStartTime: {entity:?} {start_time:?}");
        commands
            .entity(entity)
            .insert(RollbackAudioPlayerStartTime(start_time));
    }
}

/// Automatically adds [`bevy_ggrs::Rollback`] to [`RollbackAudioPlayer`]s that are missing it.
#[cfg(feature = "bevy_ggrs")]
fn add_rollback_to_rollback_sounds(
    mut commands: Commands,
    mut rollback_audio_players: Query<
        Entity,
        (With<RollbackAudioPlayer>, Without<bevy_ggrs::Rollback>),
    >,
) {
    for entity in rollback_audio_players.iter_mut() {
        use bevy_ggrs::AddRollbackCommandExtension;
        debug!("adding ggrs rollback to audio player: {entity:?}");
        commands.entity(entity).add_rollback();
    }
}

pub fn remove_finished_sounds(
    rollback_audio_players: Query<(
        Entity,
        &RollbackAudioPlayer,
        &RollbackAudioPlayerStartTime,
        Option<&PlaybackSettings>,
    )>,
    mut commands: Commands,
    audio_sources: Res<Assets<AudioSource>>,
    time: Res<Time>,
    mut durations: Local<HashMap<Handle<AudioSource>, Duration>>,
) {
    for (entity, player, start_time, settings) in rollback_audio_players.iter() {
        if let Some(audio_source) = audio_sources.get(&player.0 .0) {
            use bevy::audio::Source;

            // perf: cache duration instead of calculating every frame
            let duration = durations
                .entry(player.0.0.clone_weak())
                .or_insert_with(|| {
                    // if the duration is not cached, we calculate it
                    audio_source
                        .decoder()
                        .total_duration()
                        .unwrap_or_else(|| {
                            const FALLBACK_DURATION_SECS: u64 = 10;
                            warn!(
                                "Audio source {:?} has no total duration, defaulting to {} seconds. Make sure you use a format that supports querying duration.",
                                player.0.0,
                                FALLBACK_DURATION_SECS
                            );
                            Duration::from_secs(FALLBACK_DURATION_SECS)
                        })
                });

            let time_played = time.elapsed() - start_time.0;

            let speed = settings.map_or(1.0, |s| s.speed);
            let scaled_duration = duration.div_f32(speed);

            if time_played >= scaled_duration {
                trace!("handling finished sound: {:?} {:?}", entity, player.0 .0);
                let mode = settings.map_or(PlaybackMode::Once, |s| s.mode);

                match mode {
                    PlaybackMode::Despawn => commands.entity(entity).despawn(),
                    PlaybackMode::Remove => {
                        commands.entity(entity).remove::<(
                            RollbackAudioPlayer,
                            RollbackAudioPlayerStartTime,
                            PlaybackSettings,
                        )>();
                    }
                    // if we just leave it alone, it will continue existing in both rollback and regular version
                    PlaybackMode::Once => {}
                    PlaybackMode::Loop => {
                        // if the sound is looping, we don't despawn it, but we can reset the start time
                        // which will change the desired state and trigger a new sound to be played
                        commands
                            .entity(entity)
                            .insert(RollbackAudioPlayerStartTime(time.elapsed()));
                    }
                }
            }
        }
    }
}
