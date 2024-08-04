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
  - [ ] Sub-States
  - [ ] Computed states
  - [ ] Roll-safe events
- [x] FrameCount
- [ ] Events

## States

Bevy states when added through `app.add_state::<FooState>()` have two big problems:

1. They happen in the `StateTransition` schedule within the `MainSchedule`
2. If rolled back to the first frame, `OnEnter(InitialState)` is not re-run.

This crate provides an extension method, `add_roll_state::<S>(schedule)`, which lets you add a state to the schedule you want, and a resource, `InitialStateEntered<S>` which can be rolled back and tracks whether the initial `OnEnter` should be run (or re-run on rollbacks to the initial frame).

See the [`states`](https://github.com/johanhelsing/bevy_roll_safe/blob/main/examples/states.rs) example for usage with [`bevy_ggrs`].

## Cargo features

- `bevy_ggrs`: Enable integration with [`bevy_ggrs`]
- `math_determinism`: Enable cross-platform determinism for operations on Bevy's (`glam`) math types.

## Bevy Version Support

|bevy|bevy_roll_safe|
|----|--------------|
|0.13|0.2, main     |
|0.12|0.1           |

## License

`bevy_roll_safe` is dual-licensed under either

- MIT License (./LICENSE-MIT or <http://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 (./LICENSE-APACHE or <http://www.apache.org/licenses/LICENSE-2.0>)

at your option.

## Contributions

PRs welcome!

[`bevy_ggrs`]: https://github.com/gschup/bevy_ggrs
