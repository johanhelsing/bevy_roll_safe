use std::time::Duration;

use bevy::{app::ScheduleRunnerPlugin, log::LogPlugin, prelude::*};
use bevy_ggrs::{
    ggrs::{Config, PlayerHandle, PlayerType, SessionBuilder},
    prelude::*,
    GgrsAppExtension,
};
use bevy_roll::prelude::*;

#[derive(Debug)]
pub struct GgrsConfig;
impl Config for GgrsConfig {
    type Input = u8;
    type State = u8;
    type Address = String;
}

pub fn input(_handle: In<PlayerHandle>) -> u8 {
    0
}

#[derive(States, Reflect, Hash, Default, Debug, Eq, PartialEq, Clone)]
pub enum GameplayState {
    #[default]
    InRound,
    GameOver,
}

/// Player health. Implements and reflects Hash so it will be used in the ggrs state checksums
#[derive(Component, Reflect, Hash, Debug)]
#[reflect(Component, Hash)]
pub struct Health(u32);

impl Default for Health {
    fn default() -> Self {
        Self(10)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = SessionBuilder::<GgrsConfig>::new()
        .with_num_players(1)
        // each frame, roll back and resimulate 5 frames back in time, and compare checksums
        .with_check_distance(5);
    session = session.add_player(PlayerType::Local, 0)?;
    let session = session.start_synctest_session()?;

    App::new()
        .add_ggrs_plugin(
            GgrsPlugin::<GgrsConfig>::new()
                .with_update_frequency(60)
                .with_input_system(input)
                .register_rollback_component::<Health>()
                // Register the state's resources so GGRS can roll them back
                .register_roll_state::<GameplayState>(),
        )
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 10.0,
            ))),
            LogPlugin::default(),
        ))
        // Add the state to a specific schedule, in this case the GgrsSchedule
        .add_roll_state::<GameplayState>(GgrsSchedule)
        .add_systems(OnEnter(GameplayState::InRound), spawn_player)
        .add_systems(OnEnter(GameplayState::GameOver), log_game_over)
        .add_systems(
            GgrsSchedule,
            decrease_health
                .after(apply_state_transition::<GameplayState>)
                .run_if(in_state(GameplayState::InRound)),
        )
        .insert_resource(Session::SyncTest(session))
        .run();

    Ok(())
}

fn spawn_player(mut commands: Commands) {
    info!("spawning player");
    commands.spawn(Health::default()).add_rollback();
}

fn decrease_health(
    mut commands: Commands,
    mut players: Query<(Entity, &mut Health)>,
    mut state: ResMut<NextState<GameplayState>>,
) {
    let (player_entity, mut health) = players.single_mut();

    health.0 = health.0.saturating_sub(1);
    info!("{health:?}");

    if health.0 == 0 {
        commands.entity(player_entity).despawn_recursive();
        state.set(GameplayState::GameOver);
    }
}

fn log_game_over() {
    info!("you dead");
}
