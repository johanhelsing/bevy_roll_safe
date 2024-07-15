use std::time::Duration;

use bevy::{app::ScheduleRunnerPlugin, log::LogPlugin, prelude::*, utils::HashMap};
use bevy_ggrs::{
    ggrs::{PlayerType, SessionBuilder},
    prelude::*,
    LocalInputs, LocalPlayers,
};
use bevy_roll_safe::{prelude::*, run_state_transitions};

type GgrsConfig = bevy_ggrs::GgrsConfig<u8, String>;

fn read_local_input(mut commands: Commands, local_players: Res<LocalPlayers>) {
    let mut local_inputs = HashMap::new();
    for handle in &local_players.0 {
        local_inputs.insert(*handle, 0);
    }
    commands.insert_resource(LocalInputs::<GgrsConfig>(local_inputs));
}

#[derive(States, Hash, Default, Debug, Eq, PartialEq, Clone)]
enum GameplayState {
    #[default]
    InRound,
    GameOver,
}

/// Player health
#[derive(Component, Hash, Debug, Clone, Copy)]
struct Health(u32);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = SessionBuilder::<GgrsConfig>::new()
        .with_num_players(1)
        // each frame, roll back and resimulate 5 frames back in time, and compare checksums
        .with_check_distance(5)
        .add_player(PlayerType::Local, 0)?
        .start_synctest_session()?;

    App::new()
        .add_plugins(GgrsPlugin::<GgrsConfig>::default())
        .set_rollback_schedule_fps(60)
        .add_systems(ReadInputs, read_local_input)
        .rollback_component_with_copy::<Health>()
        .checksum_component_with_hash::<Health>()
        // Add the state transition to the ggrs schedule and register it for rollback
        .init_ggrs_state::<GameplayState>()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / 10.0,
            ))),
            LogPlugin::default(),
        ))
        .add_systems(OnEnter(GameplayState::InRound), spawn_player)
        .add_systems(OnEnter(GameplayState::GameOver), log_game_over)
        .add_systems(
            GgrsSchedule,
            decrease_health
                .after(run_state_transitions)
                .run_if(in_state(GameplayState::InRound)),
        )
        .insert_resource(Session::SyncTest(session))
        .run();

    Ok(())
}

fn spawn_player(mut commands: Commands) {
    info!("spawning player");
    commands.spawn(Health(10)).add_rollback();
}

fn decrease_health(
    mut commands: Commands,
    mut players: Query<(Entity, &mut Health)>,
    mut state: ResMut<NextState<GameplayState>>,
) {
    // this system should never run in the GameOver state,
    // so single_mut is safe to use
    let (player_entity, mut health) = players.single_mut();

    health.0 = health.0.saturating_sub(1);
    info!("{health:?}");

    if health.0 == 0 {
        info!("despawning player, setting GameOver state");
        commands.entity(player_entity).despawn_recursive();
        state.set(GameplayState::GameOver);
    }
}

fn log_game_over() {
    info!("you dead");
}
