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

/// Represents the target state for a sound effect
#[derive(Component, Clone)]
pub struct RollbackAudioPlayer {
    /// The actual sound effect to play
    pub audio_player: AudioPlayer,
}

impl From<AudioPlayer> for RollbackAudioPlayer {
    fn from(audio_player: AudioPlayer) -> Self {
        Self { audio_player }
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
                        audio_source: player.audio_player.0.clone(),
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

        let settings = settings.unwrap_or(&PlaybackSettings::ONCE);

        assert!(
            matches!(settings.mode, PlaybackMode::Once),
            "Only PlaybackMode::Once is supported for RollbackAudioPlayer"
        );

        commands.spawn((
            AudioPlayer::new(sound.audio_source.clone()),
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
    for entity in rollback_audio_players.iter_mut() {
        commands
            .entity(entity)
            .insert(RollbackAudioPlayerStartTime(time.elapsed()));
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
) {
    for (entity, player, start_time, settings) in rollback_audio_players.iter() {
        if let Some(audio_source) = audio_sources.get(&player.audio_player.0) {
            use bevy::audio::Source;

            // perf: cache duration instead of calculating every frame?
            let duration = audio_source
                .decoder()
                .total_duration()
                .unwrap_or_else(|| {
                    const FALLBACK_DURATION_SECS: u64 = 10;
                    warn!(
                        "Audio source {:?} has no total duration, defaulting to {} seconds. Make sure you use a format that supports querying duration.",
                        player.audio_player.0,
                        FALLBACK_DURATION_SECS
                    );
                    std::time::Duration::from_secs(FALLBACK_DURATION_SECS)
                });

            let time_played = time.elapsed() - start_time.0;

            let speed = settings.map_or(1.0, |s| s.speed);
            let scaled_duration = duration.div_f32(speed);

            if time_played >= scaled_duration {
                debug!("despawning finished sound: {:?}", player.audio_player.0);
                commands.entity(entity).despawn();
            }
        }
    }
}
