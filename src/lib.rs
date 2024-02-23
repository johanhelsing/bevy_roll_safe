use std::marker::PhantomData;

use bevy::{
    ecs::schedule::{run_enter_schedule, ScheduleLabel},
    prelude::*,
};

mod frame_count;

// re-exports
pub use frame_count::{increase_frame_count, RollFrameCount};

pub mod prelude {
    pub use super::RollApp;
}

pub trait RollApp {
    /// Add state transitions to the given schedule
    fn add_roll_state<S: States + FromWorld>(&mut self, schedule: impl ScheduleLabel) -> &mut Self;

    #[cfg(feature = "bevy_ggrs")]
    /// Register this state to be rolled back by bevy_ggrs
    fn add_ggrs_state<S: States + FromWorld + Clone>(&mut self) -> &mut Self;

    #[cfg(feature = "bevy_ggrs")]
    /// Register this state to be rolled back by bevy_ggrs in the specified schedule
    fn add_ggrs_state_to_schedule<S: States + FromWorld + Clone>(
        &mut self,
        schedule: impl ScheduleLabel,
    ) -> &mut Self;
}

impl RollApp for App {
    fn add_roll_state<S: States + FromWorld>(&mut self, schedule: impl ScheduleLabel) -> &mut Self {
        self.init_resource::<NextState<S>>()
            .init_resource::<State<S>>()
            .init_resource::<InitialStateEntered<S>>()
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
            )
    }

    #[cfg(feature = "bevy_ggrs")]
    fn add_ggrs_state<S: States + FromWorld + Clone>(&mut self) -> &mut Self {
        use bevy_ggrs::GgrsSchedule;
        self.add_ggrs_state_to_schedule::<S>(GgrsSchedule)
    }

    #[cfg(feature = "bevy_ggrs")]
    fn add_ggrs_state_to_schedule<S: States + FromWorld + Clone>(
        &mut self,
        schedule: impl ScheduleLabel,
    ) -> &mut Self {
        use crate::ggrs_support::{NextStateStrategy, StateStrategy};
        use bevy_ggrs::{CloneStrategy, ResourceSnapshotPlugin};

        self.add_roll_state::<S>(schedule).add_plugins((
            ResourceSnapshotPlugin::<StateStrategy<S>>::default(),
            ResourceSnapshotPlugin::<NextStateStrategy<S>>::default(),
            ResourceSnapshotPlugin::<CloneStrategy<InitialStateEntered<S>>>::default(),
        ))
    }
}

#[cfg(feature = "bevy_ggrs")]
mod ggrs_support {
    use bevy::prelude::*;
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
    impl<S: States> Strategy for NextStateStrategy<S> {
        type Target = NextState<S>;
        type Stored = Option<S>;

        fn store(target: &Self::Target) -> Self::Stored {
            target.0.to_owned()
        }

        fn load(stored: &Self::Stored) -> Self::Target {
            NextState(stored.to_owned())
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
