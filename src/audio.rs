use bevy::{platform::collections::HashSet, prelude::*};
#[cfg(feature = "bevy_ggrs")]
use bevy_ggrs::RollbackApp;

use crate::RollbackPreUpdate;

/// Plugin for managing rollback audio effects in a Bevy application.
pub struct RollbackAudioPlugin;

impl Plugin for RollbackAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, sync_rollback_sounds);
        app.add_systems(RollbackPreUpdate, remove_finished_sounds);

        #[cfg(feature = "bevy_ggrs")]
        {
            app.rollback_component_with_clone::<RollbackAudioPlayer>();
            // app.add_systems(bevy_ggrs::GgrsSchedule, remove_finished_sounds);
        }
    }
}

/// Represents the target state for a sound effect
#[derive(Component, Clone)]
pub struct RollbackAudioPlayer {
    /// The actual sound effect to play
    pub audio_player: AudioPlayer,
    /// When the sound effect should have started playing
    pub start_frame: i32,
    /// Differentiates several unique instances of the same sound playing at once.
    pub key: usize,
}

impl RollbackAudioPlayer {
    /// Creates a new RollbackAudioPlayer with the given audio player, start frame, and key.
    pub fn new(audio_player: AudioPlayer, start_frame: i32, key: usize) -> Self {
        Self {
            audio_player,
            start_frame,
            key,
        }
    }
}

/// Represents an instance of a rollback sound effect that is currently playing
#[derive(Component)]
pub struct RollbackAudioPlayerInstance {
    key: usize,
    desired_start_frame: i32,
}

impl PartialEq for RollbackAudioPlayer {
    fn eq(&self, other: &Self) -> bool {
        // TODO: make a PR for bevy to derive PartialEq for AudioPlayer
        // and remove this manual implementation
        self.audio_player.0 == other.audio_player.0
            && self.start_frame == other.start_frame
            && self.key == other.key
    }
}

impl std::hash::Hash for RollbackAudioPlayer {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // TODO: make a PR for bevy to derive Hash for AudioPlayer
        // and remove this manual implementation
        self.audio_player.0.hash(state);
        self.start_frame.hash(state);
        self.key.hash(state);
    }
}

impl std::cmp::Eq for RollbackAudioPlayer {}

/// Updates playing sounds to match the desired state
/// spawns any missing sounds that should be playing.
/// and despawns any sounds that should not be playing.
fn sync_rollback_sounds(
    mut commands: Commands,
    rollback_audio_players: Query<&RollbackAudioPlayer>,
    instances: Query<(Entity, &RollbackAudioPlayerInstance, &AudioPlayer)>,
) {
    // TODO: sound effects that have actually finished playing should be
    // despawned from the rollback world as well. How to do that?

    let desired_state: HashSet<RollbackAudioPlayer> =
        rollback_audio_players.iter().cloned().collect();

    let mut playing_sounds = HashSet::new();

    for (instance_entity, instance, audio_player) in &instances {
        // reconstruct the RollbackSoundEffect from the playing sound
        let rollback_sound = RollbackAudioPlayer {
            audio_player: audio_player.clone(),
            start_frame: instance.desired_start_frame,
            key: instance.key,
        };

        // if the playing sound is not in the desired state, despawn it
        if !desired_state.contains(&rollback_sound) {
            commands.entity(instance_entity).despawn();
        } else {
            playing_sounds.insert(rollback_sound);
        }
    }

    // spawn any missing sounds
    for sound in desired_state.difference(&playing_sounds) {
        info!("Spawning sound: {:?}", sound.audio_player.0);
        commands.spawn((
            sound.audio_player.clone(),
            RollbackAudioPlayerInstance {
                key: sound.key,
                desired_start_frame: sound.start_frame,
            },
            // TODO: handle other settings as well
            PlaybackSettings::ONCE,
        ));
    }
}

pub fn remove_finished_sounds(
    frame: Res<bevy_ggrs::RollbackFrameCount>,
    rollback_audio_players: Query<(Entity, &RollbackAudioPlayer)>,
    mut commands: Commands,
    audio_sources: Res<Assets<AudioSource>>,
    frame_rate: Res<bevy_ggrs::RollbackFrameRate>,
) {
    for (entity, player) in rollback_audio_players.iter() {
        if let Some(audio_source) = audio_sources.get(&player.audio_player.0) {
            use bevy::audio::Source;

            let frames_played = frame.0 - player.start_frame;

            // perf: cache frames_to_play instead of calculating every frame?
            let seconds_to_play = audio_source
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
                })
                .as_secs_f64();

            let frames_to_play = (seconds_to_play * (**frame_rate) as f64) as i32;

            if frames_played >= frames_to_play {
                info!("despawning finished sound: {:?}", player.audio_player.0);
                commands.entity(entity).despawn();
            }
        }
    }
}
