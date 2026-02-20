# bevy_roll_safe

[![crates.io](https://img.shields.io/crates/v/bevy_roll_safe.svg)](https://crates.io/crates/bevy_roll_safe)
![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)
[![docs.rs](https://img.shields.io/docsrs/bevy_roll_safe)](https://docs.rs/bevy_roll_safe)

Rollback-safe implementations and utilities for Bevy Engine.

## Motivation

Some of Bevy's features can't be used in a rollback context (with crates such as [`bevy_ggrs`]). This is either because they behave non-deterministically, rely on inaccessible local system state, or are tightly coupled to the `Main` schedule.

## Roadmap

- [x] States
  - [x] Basic freely mutable states
  - [x] `OnEnter`/`OnLeave`/`OnTransition`
- [x] FrameCount
- [x] Rollback-safe "Main"/default schedules
- [x] Audio playback
  - [x] Support all `PlaybackMode`s
  - [ ] Support for seeking in "time-critical" audio
  - [ ] Support for formats that don't report sound durations (mp3/ogg)
- [ ] Events

## States

Bevy states when added through `app.init_state::<FooState>()` have two big problems:

1. They happen in the `StateTransition` schedule within the `MainSchedule`
2. If rolled back to the first frame, `OnEnter(InitialState)` is not re-run.

This crate provides an extension method, `init_roll_state_in_schedule::<S>(schedule)`, which lets you add a state to the schedule you want, and a resource, `InitialStateEntered<S>` which can be rolled back and tracks whether the initial `OnEnter` should be run (or re-run on rollbacks to the initial frame).

If you are using the rollback schedule plugin as well. Adding a rollback safe state is a simple as `app.init_roll_state::<YourState>()`.

See the [`states`](https://github.com/johanhelsing/bevy_roll_safe/blob/main/examples/states.rs) example for usage with [`bevy_ggrs`].

## Default rollback schedule

`RollbackSchedulePlugin` adds rollback-specific alternatives to the schedules in Bevy's `FixedMain`/`Main` schedules.

The plugin takes a parent schedule as input, so it can easily be added to the ggrs schedule or any other schedule you want.

## Rollback audio

`RollbackAudioPlugin` lets you easily play sound effects from a rollback world without duplicate sounds playing over each other. It depends on the `RollbackSchedulePlugin`, or you need to add the maintenance system in a similar order to your own schedules.

## Cargo features

- `audio`: Enable rollback-safe wrapper for `bevy_audio`
- `bevy_ggrs`: Enable integration with [`bevy_ggrs`]
- `math_determinism`: Enable cross-platform determinism for operations on Bevy's (`glam`) math types.

## Bevy Version Support

|bevy|bevy_roll_safe|
|----|--------------|
|0.18|0.7, main     |
|0.17|0.6,          |
|0.16|0.5           |
|0.15|0.4           |
|0.14|0.3           |
|0.13|0.2           |
|0.12|0.1           |

## License

`bevy_roll_safe` is dual-licensed under either

- MIT License (./LICENSE-MIT or <http://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 (./LICENSE-APACHE or <http://www.apache.org/licenses/LICENSE-2.0>)

at your option.

## Contributions

PRs welcome!

[`bevy_ggrs`]: https://github.com/gschup/bevy_ggrs
