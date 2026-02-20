#![doc = include_str!("../README.md")]

use std::marker::PhantomData;

use bevy::{ecs::schedule::ScheduleLabel, prelude::*, state::state::FreelyMutableState};

#[cfg(feature = "audio")]
mod audio;
mod frame_count;
mod schedule;

// re-exports
#[cfg(feature = "audio")]
pub use audio::{
    remove_finished_sounds, start_rollback_sounds, sync_rollback_sounds, RollbackAudioPlayer,
    RollbackAudioPlayerInstance, RollbackAudioPlugin,
};
pub use frame_count::{increase_frame_count, RollFrameCount};
pub use schedule::{
    RollbackPostUpdate, RollbackPreUpdate, RollbackSchedulePlugin, RollbackStateTransition,
    RollbackUpdate,
};

pub mod prelude {
    pub use super::{
        RollApp, RollbackPostUpdate, RollbackPreUpdate, RollbackSchedulePlugin,
        RollbackStateTransition, RollbackUpdate,
    };
    #[cfg(feature = "audio")]
    pub use super::{RollbackAudioPlayer, RollbackAudioPlugin};
}

pub trait RollApp {
    /// Init state transitions in the given schedule
    fn init_roll_state_in_schedule<S: States + FromWorld + FreelyMutableState>(
        &mut self,
        schedule: impl ScheduleLabel,
    ) -> &mut Self;

    /// Init state transitions in the given schedule
    fn init_roll_state<S: States + FromWorld + FreelyMutableState>(&mut self) -> &mut Self;

    #[cfg(feature = "bevy_ggrs")]
    /// Register this state to be rolled back by bevy_ggrs
    fn init_ggrs_state<S: States + FromWorld + Clone + FreelyMutableState>(&mut self) -> &mut Self;

    #[cfg(feature = "bevy_ggrs")]
    /// Register this state to be rolled back by bevy_ggrs in the specified schedule
    fn init_ggrs_state_in_schedule<S: States + FromWorld + Clone + FreelyMutableState>(
        &mut self,
        schedule: impl ScheduleLabel,
    ) -> &mut Self;
}

impl RollApp for App {
    fn init_roll_state_in_schedule<S: States + FromWorld + FreelyMutableState>(
        &mut self,
        schedule: impl ScheduleLabel,
    ) -> &mut Self {
        if !self.world().contains_resource::<State<S>>() {
            self.init_resource::<State<S>>()
                .init_resource::<NextState<S>>()
                .init_resource::<InitialStateEntered<S>>()
                // .add_event::<StateTransitionEvent<S>>()
                .add_systems(
                    schedule,
                    (
                        run_enter_schedule::<S>
                            .run_if(resource_equals(InitialStateEntered::<S>(false, default()))),
                        mark_state_initialized::<S>
                            .run_if(resource_equals(InitialStateEntered::<S>(false, default()))),
                        apply_state_transition::<S>,
                    )
                        .chain(),
                );
        } else {
            let name = std::any::type_name::<S>();
            warn!("State {} is already initialized.", name);
        }

        self
    }

    fn init_roll_state<S: States + FromWorld + FreelyMutableState>(&mut self) -> &mut Self {
        self.init_roll_state_in_schedule::<S>(RollbackStateTransition)
    }

    #[cfg(feature = "bevy_ggrs")]
    fn init_ggrs_state<S: States + FromWorld + Clone + FreelyMutableState>(&mut self) -> &mut Self {
        // verify the schedule exists first?
        self.get_schedule(RollbackStateTransition)
            .unwrap_or_else(|| {
                panic!(
                    "RollbackStateTransition schedule does not exist. \
                     Please add it by adding the `RollbackSchedulePlugin` \
                     or call `init_ggrs_state_in_schedule` with the desired schedule."
                )
            });

        self.init_ggrs_state_in_schedule::<S>(RollbackStateTransition)
    }

    #[cfg(feature = "bevy_ggrs")]
    fn init_ggrs_state_in_schedule<S: States + FromWorld + Clone + FreelyMutableState>(
        &mut self,
        schedule: impl ScheduleLabel,
    ) -> &mut Self {
        use crate::ggrs_support::{NextStateStrategy, StateStrategy};
        use bevy_ggrs::{CloneStrategy, ResourceSnapshotPlugin};

        self.init_roll_state_in_schedule::<S>(schedule)
            .add_plugins((
                ResourceSnapshotPlugin::<StateStrategy<S>>::default(),
                ResourceSnapshotPlugin::<NextStateStrategy<S>>::default(),
                ResourceSnapshotPlugin::<CloneStrategy<InitialStateEntered<S>>>::default(),
            ))
    }
}

#[cfg(feature = "bevy_ggrs")]
mod ggrs_support {
    use bevy::{prelude::*, state::state::FreelyMutableState};
    use bevy_ggrs::Strategy;
    use std::marker::PhantomData;

    pub(crate) struct StateStrategy<S: States>(PhantomData<S>);

    // todo: make State<S> implement clone instead
    impl<S: States> Strategy for StateStrategy<S> {
        type Target = State<S>;
        type Stored = S;

        fn store(target: &Self::Target) -> Self::Stored {
            target.get().to_owned()
        }

        fn load(stored: &Self::Stored) -> Self::Target {
            State::new(stored.to_owned())
        }
    }

    pub(crate) struct NextStateStrategy<S: States>(PhantomData<S>);

    // todo: make NextState<S> implement clone instead
    impl<S: States + FreelyMutableState> Strategy for NextStateStrategy<S> {
        type Target = NextState<S>;
        type Stored = Option<S>;

        fn store(target: &Self::Target) -> Self::Stored {
            match target {
                NextState::Unchanged => None,
                NextState::Pending(s) | NextState::PendingIfNeq(s) => Some(s.to_owned()),
            }
        }

        fn load(stored: &Self::Stored) -> Self::Target {
            match stored {
                None => NextState::Unchanged,
                Some(s) => NextState::Pending(s.to_owned()),
            }
        }
    }
}

#[derive(Resource, Debug, Reflect, Eq, PartialEq, Clone)]
#[reflect(Resource)]
pub struct InitialStateEntered<S: States>(bool, PhantomData<S>);

impl<S: States> Default for InitialStateEntered<S> {
    fn default() -> Self {
        Self(false, default())
    }
}

fn mark_state_initialized<S: States + FromWorld>(
    mut state_initialized: ResMut<InitialStateEntered<S>>,
) {
    state_initialized.0 = true;
}

/// Run the enter schedule (if it exists) for the current state.
pub fn run_enter_schedule<S: States>(world: &mut World) {
    let Some(state) = world.get_resource::<State<S>>() else {
        return;
    };
    world.try_run_schedule(OnEnter(state.get().clone())).ok();
}

/// If a new state is queued in [`NextState<S>`], this system:
/// - Takes the new state value from [`NextState<S>`] and updates [`State<S>`].
/// - Sends a relevant [`StateTransitionEvent`]
/// - Runs the [`OnExit(exited_state)`] schedule, if it exists.
/// - Runs the [`OnTransition { from: exited_state, to: entered_state }`](OnTransition), if it exists.
/// - Runs the [`OnEnter(entered_state)`] schedule, if it exists.
pub fn apply_state_transition<S: States + FreelyMutableState>(world: &mut World) {
    // We want to take the `NextState` resource,
    // but only mark it as changed if it wasn't empty.
    let Some(mut next_state_resource) = world.get_resource_mut::<NextState<S>>() else {
        return;
    };
    if let NextState::Pending(entered) = next_state_resource.bypass_change_detection() {
        let entered = entered.clone();
        *next_state_resource = NextState::Unchanged;
        match world.get_resource_mut::<State<S>>() {
            Some(mut state_resource) => {
                if *state_resource != entered {
                    let exited = state_resource.get().clone();
                    *state_resource = State::new(entered.clone());
                    // world.send_event(StateTransitionEvent {
                    //     exited: Some(exited.clone()),
                    //     entered: Some(entered.clone()),
                    // });
                    // Try to run the schedules if they exist.
                    world.try_run_schedule(OnExit(exited.clone())).ok();
                    world
                        .try_run_schedule(OnTransition {
                            exited,
                            entered: entered.clone(),
                        })
                        .ok();
                    world.try_run_schedule(OnEnter(entered)).ok();
                }
            }
            None => {
                world.insert_resource(State::new(entered.clone()));
                world.try_run_schedule(OnEnter(entered)).ok();
            }
        };
    }
}
