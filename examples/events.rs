use bevy::{
    app::ScheduleRunnerPlugin,
    ecs::schedule::{LogLevel, ScheduleBuildSettings, ScheduleLabel},
    log::LogPlugin,
    prelude::*,
    utils::HashMap,
};
use bevy_ggrs::{
    ggrs::{PlayerType, SessionBuilder},
    prelude::*,
    LocalInputs, LocalPlayers, RollbackFrameCount,
};
use bevy_roll_safe::prelude::*;
use std::time::Duration;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

type GgrsConfig = bevy_ggrs::GgrsConfig<u8, String>;

#[derive(Event, Clone)]
struct DiedEvent;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash, EnumIter)]
enum GameplaySchedule {
    First,
    Update,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = SessionBuilder::<GgrsConfig>::new()
        .with_num_players(1)
        // each frame, roll back and resimulate 5 frames back in time, and compare checksums
        .with_check_distance(2)
        .add_player(PlayerType::Local, 0)?
        .start_synctest_session()?;

    let mut app = App::new();

    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 10.0,
        ))),
        LogPlugin::default(),
    ))
    .add_plugins(GgrsPlugin::<GgrsConfig>::default())
    .set_rollback_schedule_fps(60)
    .add_systems(ReadInputs, read_local_input)
    .add_systems(GgrsSchedule, run_gameplay_schedules)
    .rollback_component_with_copy::<Health>()
    .checksum_component_with_hash::<Health>()
    .rollback_resource_with_copy::<Deaths>()
    .checksum_resource_with_hash::<Deaths>()
    // add the event, and tell ggrs it should be rolled back and updated in the specified schedule
    .add_ggrs_event::<DiedEvent>(GameplaySchedule::First)
    .add_systems(GameplaySchedule::Update, decrease_health)
    .add_systems(GameplaySchedule::Update, handle_died.after(decrease_health))
    .add_systems(Startup, spawn_player)
    .init_resource::<Deaths>()
    .insert_resource(Session::SyncTest(session));

    // todo: maybe add GamePlaySchedule to the library?
    for label in GameplaySchedule::iter() {
        app.edit_schedule(label, |schedule| {
            schedule.set_build_settings(ScheduleBuildSettings {
                ambiguity_detection: LogLevel::Error,
                ..default()
            });
        });
    }

    app.run();

    Ok(())
}

/// Player health
#[derive(Component, Hash, Debug, Clone, Copy)]
struct Health(u32);

/// How many deaths we've seen
#[derive(Resource, Hash, Debug, Clone, Copy, Default, Deref, DerefMut)]
struct Deaths(u32);

fn read_local_input(mut commands: Commands, local_players: Res<LocalPlayers>) {
    let mut local_inputs = HashMap::new();
    for handle in &local_players.0 {
        local_inputs.insert(*handle, 0);
    }
    commands.insert_resource(LocalInputs::<GgrsConfig>(local_inputs));
}

fn run_gameplay_schedules(world: &mut World) {
    for schedule in GameplaySchedule::iter() {
        let _ = world.try_run_schedule(schedule);
    }
}

fn spawn_player(mut commands: Commands) {
    info!("spawning player");
    commands.spawn(Health(10)).add_rollback();
}

fn decrease_health(
    mut commands: Commands,
    mut players: Query<(Entity, &mut Health)>,
    mut died_events: RollEventWriter<DiedEvent>,
) {
    for (player_entity, mut health) in &mut players {
        health.0 = health.0.saturating_sub(1);
        info!("{health:?}");

        if health.0 == 0 {
            info!("despawning player, sending died event");
            commands.entity(player_entity).despawn_recursive();
            died_events.send(DiedEvent);
        }
    }
}

fn handle_died(
    mut died_events: RollEventReader<DiedEvent>,
    mut deaths: ResMut<Deaths>,
    frame: Res<RollbackFrameCount>,
) {
    for _ in died_events.read() {
        **deaths += 1;
        info!("Died event received in frame {frame:?}, deaths {deaths:?}");
    }
}
