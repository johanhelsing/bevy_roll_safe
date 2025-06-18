use bevy::{platform::collections::HashSet, prelude::*};
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
// /// Differentiates several unique instances of the same sound playing at once.
// pub key: usize,

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

struct PlayingRollbackAudioKey {
    player: AudioPlayer,
    start_time: Duration,
}

impl PartialEq for PlayingRollbackAudioKey {
    fn eq(&self, other: &Self) -> bool {
        // TODO: make a PR for bevy to derive PartialEq for AudioPlayer
        // and remove this manual implementation
        self.player.0 == other.player.0 && self.start_time == other.start_time
    }
}

impl std::hash::Hash for PlayingRollbackAudioKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // TODO: make a PR for bevy to derive Hash for AudioPlayer
        // and remove this manual implementation
        self.player.0.hash(state);
        self.start_time.hash(state);
    }
}

impl std::cmp::Eq for PlayingRollbackAudioKey {}

/// Updates playing sounds to match the desired state
/// spawns any missing sounds that should be playing.
/// and despawns any sounds that should not be playing.
pub fn sync_rollback_sounds(
    mut commands: Commands,
    rollback_audio_players: Query<(&RollbackAudioPlayer, &RollbackAudioPlayerStartTime)>,
    instances: Query<(Entity, &RollbackAudioPlayerInstance, &AudioPlayer)>,
) {
    let desired_state: HashSet<PlayingRollbackAudioKey> = rollback_audio_players
        .iter()
        .map(|(player, start_time)| PlayingRollbackAudioKey {
            player: player.audio_player.clone(),
            start_time: start_time.0,
        })
        .collect();

    let mut playing_sounds = HashSet::new();

    for (instance_entity, instance, audio_player) in &instances {
        let rollback_sound_key = PlayingRollbackAudioKey {
            player: audio_player.clone(),
            start_time: instance.desired_start_time,
        };

        // if the playing sound is not in the desired state, despawn it
        if !desired_state.contains(&rollback_sound_key) {
            commands.entity(instance_entity).despawn();
        } else {
            playing_sounds.insert(rollback_sound_key);
        }
    }

    // spawn any missing sounds
    for sound in desired_state.difference(&playing_sounds) {
        info!("Spawning sound: {:?}", sound.player.0);
        commands.spawn((
            sound.player.clone(),
            RollbackAudioPlayerInstance {
                desired_start_time: sound.start_time,
            },
            // TODO: handle other settings as well
            PlaybackSettings::ONCE,
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
    rollback_audio_players: Query<(Entity, &RollbackAudioPlayer, &RollbackAudioPlayerStartTime)>,
    mut commands: Commands,
    audio_sources: Res<Assets<AudioSource>>,
    time: Res<Time>,
) {
    for (entity, player, start_time) in rollback_audio_players.iter() {
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

            if time_played >= duration {
                debug!("despawning finished sound: {:?}", player.audio_player.0);
                commands.entity(entity).despawn();
            }
        }
    }
}
