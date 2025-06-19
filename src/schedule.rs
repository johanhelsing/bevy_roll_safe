use bevy::{
    ecs::schedule::{
        ExecutorKind, InternedScheduleLabel, LogLevel, ScheduleBuildSettings, ScheduleLabel,
    },
    prelude::*,
};

/// Runs rollback-safe state transitions
///
/// By default, it will be triggered each frame after [`RollbackPreUpdate`], but
/// you can manually trigger it at arbitrary times by creating an exclusive
/// system to run the schedule.
///
/// ```rust
/// use bevy::state::prelude::*;
/// use bevy::ecs::prelude::*;
/// use bevy_roll_safe::prelude::*;
///
/// fn run_state_transitions(world: &mut World) {
///     let _ = world.try_run_schedule(RollbackStateTransition);
/// }
/// ```
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash, Default)]

pub struct RollbackStateTransition;

/// The schedule that contains logic that must run before [`RollbackUpdate`]. For example, a system that reads raw keyboard
/// input OS events into an `Events` resource. This enables systems in [`RollbackUpdate`] to consume the events from the `Events`
/// resource without actually knowing about (or taking a direct scheduler dependency on) the "os-level keyboard event system".
///
/// [`RollbackPreUpdate`] exists to do "engine/plugin preparation work" that ensures the APIs consumed in [`RollbackUpdate`] are "ready".
/// [`RollbackPreUpdate`] abstracts out "pre work implementation details".
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct RollbackPreUpdate;

/// The schedule that contains most gameplay logic
///
/// See the [`RollbackUpdate`] schedule for examples of systems that *should not* use this schedule.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct RollbackUpdate;

/// The schedule that contains logic that must run after [`RollbackUpdate`].
///
/// [`RollbackPostUpdate`] exists to do "engine/plugin response work" to things that happened in [`RollbackUpdate`].
/// [`RollbackPostUpdate`] abstracts out "implementation details" from users defining systems in [`RollbackUpdate`].
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct RollbackPostUpdate;

pub struct RollbackSchedulePlugin {
    schedule: InternedScheduleLabel,
}

impl RollbackSchedulePlugin {
    pub fn new(schedule: impl ScheduleLabel + 'static) -> Self {
        Self {
            schedule: schedule.intern(),
        }
    }

    #[cfg(feature = "bevy_ggrs")]
    pub fn new_ggrs() -> Self {
        Self::new(bevy_ggrs::GgrsSchedule)
    }
}

impl Plugin for RollbackSchedulePlugin {
    fn build(&self, app: &mut App) {
        // simple "facilitator" schedules benefit from simpler single threaded scheduling
        let mut rollback_schedule = Schedule::new(self.schedule);
        rollback_schedule.set_executor_kind(ExecutorKind::SingleThreaded);

        for label in RollbackScheduleOrder::default().labels {
            app.edit_schedule(label, |schedule| {
                schedule.set_build_settings(ScheduleBuildSettings {
                    ambiguity_detection: LogLevel::Error,
                    ..default()
                });
            });
        }

        app.insert_resource(RollbackScheduleOrder::default())
            .add_systems(self.schedule, run_schedules);
    }
}

//TODO: expose in public API?
/// Defines the schedules to be run for the rollback schedule, including
/// their order.
#[derive(Resource, Debug)]
struct RollbackScheduleOrder {
    /// The labels to run for the main phase of the rollback schedule (in the order they will be run).
    pub labels: Vec<InternedScheduleLabel>,
}

impl Default for RollbackScheduleOrder {
    fn default() -> Self {
        Self {
            labels: vec![
                RollbackPreUpdate.intern(),
                RollbackStateTransition.intern(),
                RollbackUpdate.intern(),
                RollbackPostUpdate.intern(),
            ],
        }
    }
}

fn run_schedules(world: &mut World) {
    world.resource_scope(|world, order: Mut<RollbackScheduleOrder>| {
        for label in &order.labels {
            trace!("Running rollback schedule: {:?}", label);
            let _ = world.try_run_schedule(*label);
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::{InitialStateEntered, RollApp};

    use super::*;

    #[derive(Resource, Debug, Default)]
    struct IntResource(i32);

    fn increase_int_resource(mut int_resource: ResMut<IntResource>) {
        int_resource.0 += 1;
    }

    #[test]
    fn rollback_schedule_in_update() {
        let mut app = App::new();
        app.add_plugins(RollbackSchedulePlugin::new(Update));
        app.init_resource::<IntResource>();
        app.add_systems(RollbackUpdate, increase_int_resource);
        app.update();
        assert_eq!(
            app.world().resource::<IntResource>().0,
            1,
            "IntResource should be incremented by 1"
        );
        app.update();
        assert_eq!(
            app.world().resource::<IntResource>().0,
            2,
            "IntResource should be incremented by 1 two times"
        );
    }

    #[derive(States, Hash, Default, Debug, Eq, PartialEq, Clone)]
    enum GameplayState {
        #[default]
        InRound,
        GameOver,
    }

    #[test]
    fn add_states_to_rollback_schedule() {
        let mut app = App::new();
        app.add_plugins(RollbackSchedulePlugin::new(Update));
        app.init_resource::<IntResource>();
        app.init_roll_state::<GameplayState>();
        app.add_systems(OnEnter(GameplayState::InRound), increase_int_resource);
        assert!(app.world().contains_resource::<State<GameplayState>>());
        assert!(app.world().contains_resource::<NextState<GameplayState>>());
        assert_eq!(app.world().resource::<IntResource>().0, 0);
        assert!(
            !app.world()
                .resource::<InitialStateEntered<GameplayState>>()
                .0
        );

        // calling `update` will cause the initial state to be entered
        app.update();

        assert!(
            app.world()
                .resource::<InitialStateEntered<GameplayState>>()
                .0
        );
        assert_eq!(app.world().resource::<IntResource>().0, 1);
    }

    #[test]
    #[should_panic(expected = "RollbackStateTransition")]
    fn init_ggrs_states_without_rollback_state_transition_schedule_panics() {
        App::new().init_ggrs_state::<GameplayState>();
    }

    fn set_game_over_state(mut next_state: ResMut<NextState<GameplayState>>) {
        next_state.set(GameplayState::GameOver);
    }

    #[test]
    #[cfg(feature = "bevy_ggrs")]
    fn can_roll_back_states() {
        use bevy_ggrs::{AdvanceWorld, GgrsSchedule, LoadWorld, SaveWorld, SnapshotPlugin};

        let mut app = App::new();

        app.add_plugins(SnapshotPlugin)
            .add_plugins(RollbackSchedulePlugin::new_ggrs())
            // TODO: use `GgrsPlugin` instead of `SnapshotPlugin` and remove this
            .add_systems(AdvanceWorld, |world: &mut World| {
                dbg!("Advancing world in GgrsSchedule");
                world.try_run_schedule(GgrsSchedule).unwrap();
            })
            .add_systems(RollbackUpdate, || {
                dbg!("RollbackUpdate");
            })
            .init_resource::<IntResource>()
            .init_ggrs_state::<GameplayState>()
            .add_systems(
                RollbackUpdate,
                // go directly to GameOver state
                set_game_over_state.run_if(in_state(GameplayState::InRound)),
            );

        assert_eq!(
            *app.world().resource::<State<GameplayState>>(),
            GameplayState::InRound
        );

        assert!(matches!(
            app.world().resource::<NextState<GameplayState>>(),
            NextState::Unchanged,
        ));

        app.world_mut().run_schedule(SaveWorld);
        app.world_mut().run_schedule(AdvanceWorld);

        assert_eq!(
            *app.world().resource::<State<GameplayState>>(),
            GameplayState::InRound,
            "State should not change until the next frame"
        );

        assert!(matches!(
            app.world().resource::<NextState<GameplayState>>(),
            NextState::Pending(GameplayState::GameOver)
        ));

        app.world_mut().run_schedule(AdvanceWorld);

        assert_eq!(
            *app.world().resource::<State<GameplayState>>(),
            GameplayState::GameOver,
        );

        assert!(matches!(
            app.world().resource::<NextState<GameplayState>>(),
            NextState::Unchanged
        ));

        // Roll back to frame 0
        app.world_mut().run_schedule(LoadWorld);

        assert_eq!(
            *app.world().resource::<State<GameplayState>>(),
            GameplayState::InRound,
        );

        assert!(matches!(
            app.world().resource::<NextState<GameplayState>>(),
            NextState::Unchanged,
        ));
    }
}
