use std::marker::PhantomData;

use bevy::{ecs::schedule::ScheduleLabel, prelude::*, state::state::FreelyMutableState};

mod frame_count;

// re-exports
pub use frame_count::{increase_frame_count, RollFrameCount};

pub mod prelude {
    pub use super::RollApp;
}

pub trait RollApp {
    /// Init state transitions in the given schedule
    fn init_roll_state<S: States + FromWorld + FreelyMutableState>(
        &mut self,
        schedule: impl ScheduleLabel,
    ) -> &mut Self;

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
    fn init_roll_state<S: States + FromWorld + FreelyMutableState>(
        &mut self,
        schedule: impl ScheduleLabel,
    ) -> &mut Self {
        self.init_resource::<State<S>>()
            .init_resource::<NextState<S>>()
            .init_resource::<InitialStateEntered<S>>()
            // events are not rollback safe, but `apply_state_transition` will cause errors without it
            .add_event::<StateTransitionEvent<S>>()
            .add_systems(schedule, (run_state_transitions,).chain())
    }

    #[cfg(feature = "bevy_ggrs")]
    fn init_ggrs_state<S: States + FromWorld + Clone + FreelyMutableState>(&mut self) -> &mut Self {
        use bevy_ggrs::GgrsSchedule;
        self.init_ggrs_state_in_schedule::<S>(GgrsSchedule)
    }

    #[cfg(feature = "bevy_ggrs")]
    fn init_ggrs_state_in_schedule<S: States + FromWorld + Clone + FreelyMutableState>(
        &mut self,
        schedule: impl ScheduleLabel,
    ) -> &mut Self {
        use crate::ggrs_support::{NextStateStrategy, StateStrategy};
        use bevy_ggrs::{CloneStrategy, ResourceSnapshotPlugin};

        self.init_roll_state::<S>(schedule).add_plugins((
            ResourceSnapshotPlugin::<StateStrategy<S>>::default(),
            ResourceSnapshotPlugin::<NextStateStrategy<S>>::default(),
            ResourceSnapshotPlugin::<CloneStrategy<InitialStateEntered<S>>>::default(),
        ))
    }
}

pub fn run_state_transitions(world: &mut World) {
    let _ = world.try_run_schedule(StateTransition);
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
                NextState::Pending(s) => Some(s.to_owned()),
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
