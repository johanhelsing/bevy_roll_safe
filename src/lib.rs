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
    fn add_roll_state<S: States>(&mut self, schedule: impl ScheduleLabel) -> &mut Self;

    #[cfg(feature = "bevy_ggrs")]
    /// Register this state to be rolled back by bevy_ggrs
    fn register_ggrs_state<S: States + Clone>(&mut self) -> &mut Self;
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

    #[cfg(feature = "bevy_ggrs")]
    fn register_ggrs_state<S: States + Clone>(&mut self) -> &mut Self {
        use bevy_ggrs::{CloneStrategy, ResourceSnapshotPlugin, Strategy};
        use std::marker::PhantomData;

        self.add_plugins((
            ResourceSnapshotPlugin::<StateStrategy<S>>::default(),
            ResourceSnapshotPlugin::<NextStateStrategy<S>>::default(),
            ResourceSnapshotPlugin::<CloneStrategy<InitialStateEntered<S>>>::default(),
        ));

        struct StateStrategy<S: States>(PhantomData<S>);

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

        struct NextStateStrategy<S: States>(PhantomData<S>);
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

        self
    }
}

#[derive(Resource, Debug, Reflect, Default, Eq, PartialEq, Clone)]
#[reflect(Resource)]
pub struct InitialStateEntered<S: States>(Option<S>); // todo: PhantomData instead?

fn mark_state_initialized<S: States>(mut state_initialized: ResMut<InitialStateEntered<S>>) {
    state_initialized.0 = Some(default());
}
