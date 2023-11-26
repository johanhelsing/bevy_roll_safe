use bevy::{
    ecs::schedule::{run_enter_schedule, ScheduleLabel},
    prelude::*,
};

mod events;
mod frame_count;

use events::{roll_event_update_condition, roll_event_update_system, RollEvent, RollEvents};
// re-exports
pub use events::{RollEventReader, RollEventWriter};
pub use frame_count::{increase_frame_count, RollFrameCount};

pub mod prelude {
    pub use super::{RollApp, RollEventReader, RollEventWriter};
}

pub trait RollApp {
    /// Add state transitions to the given schedule
    fn add_roll_state<S: States>(&mut self, schedule: impl ScheduleLabel) -> &mut Self;

    /// Setup the application to manage events of type `T`.
    ///
    /// This is done by adding a [`Resource`] of type [`RollEvents::<T>`],
    /// and inserting an [`roll_event_update_system`] into the provided schedule
    ///
    /// # Examples
    ///
    /// ```
    /// # use bevy_app::prelude::*;
    /// # use bevy_ecs::prelude::*;
    /// #
    /// # #[derive(Event, Clone)]
    /// # struct MyEvent;
    /// # let mut app = App::new();
    /// #
    /// app.add_roll_event::<MyEvent>(First);
    /// ```
    ///
    /// [`event_update_system`]: bevy_ecs::event::event_update_system
    fn add_roll_event<T: RollEvent>(&mut self, schedule: impl ScheduleLabel) -> &mut Self;

    #[cfg(feature = "bevy_ggrs")]
    /// Register this state to be rolled back by bevy_ggrs
    fn add_ggrs_state<S: States + Clone>(&mut self) -> &mut Self;

    #[cfg(feature = "bevy_ggrs")]
    /// Register this state to be rolled back by bevy_ggrs in the specified schedule
    fn add_ggrs_state_to_schedule<S: States + Clone>(
        &mut self,
        schedule: impl ScheduleLabel,
    ) -> &mut Self;

    /// Register this event to be rolled back by bevy_ggrs in the specified schedule
    #[cfg(feature = "bevy_ggrs")]
    fn add_ggrs_event<T: RollEvent>(&mut self, update_in_schedule: impl ScheduleLabel)
        -> &mut Self;
}

impl RollApp for App {
    fn add_roll_state<S: States>(&mut self, schedule: impl ScheduleLabel) -> &mut Self {
        self.init_resource::<NextState<S>>()
            .init_resource::<State<S>>()
            .init_resource::<InitialStateEntered<S>>()
            .add_systems(
                schedule,
                (
                    run_enter_schedule::<S>.run_if(resource_equals(InitialStateEntered::<S>(None))),
                    mark_state_initialized::<S>
                        .run_if(resource_equals(InitialStateEntered::<S>(None))),
                    apply_state_transition::<S>,
                )
                    .chain(),
            )
    }

    fn add_roll_event<T>(&mut self, schedule: impl ScheduleLabel) -> &mut Self
    where
        T: RollEvent,
    {
        if !self.world.contains_resource::<RollEvents<T>>() {
            self.init_resource::<RollEvents<T>>().add_systems(
                schedule,
                roll_event_update_system::<T>.run_if(roll_event_update_condition::<T>),
            );
        }
        self
    }

    #[cfg(feature = "bevy_ggrs")]
    fn add_ggrs_state<S: States + Clone>(&mut self) -> &mut Self {
        use bevy_ggrs::GgrsSchedule;
        self.add_ggrs_state_to_schedule::<S>(GgrsSchedule)
    }

    #[cfg(feature = "bevy_ggrs")]
    fn add_ggrs_state_to_schedule<S: States + Clone>(
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

    // #[cfg(feature = "bevy_ggrs")]
    // fn add_ggrs_event<T: RollEvent>(&mut self) -> &mut Self {
    //     use bevy_ggrs::GgrsSchedule;
    //     self.add_ggrs_event_to_schedule::<T>(GgrsSchedule)
    // }

    #[cfg(feature = "bevy_ggrs")]
    fn add_ggrs_event<T: RollEvent>(
        &mut self,
        update_in_schedule: impl ScheduleLabel,
    ) -> &mut Self {
        use bevy_ggrs::GgrsApp;

        self.add_roll_event::<T>(update_in_schedule)
            .rollback_resource_with_clone::<RollEvents<T>>()
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

#[derive(Resource, Debug, Reflect, Default, Eq, PartialEq, Clone)]
#[reflect(Resource)]
pub struct InitialStateEntered<S: States>(Option<S>); // todo: PhantomData instead?

fn mark_state_initialized<S: States>(mut state_initialized: ResMut<InitialStateEntered<S>>) {
    state_initialized.0 = Some(default());
}
