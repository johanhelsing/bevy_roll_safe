# bevy_roll

![MIT/Apache 2.0](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)

Rollback-safe implementations/replacements for Bevy Engine.

## Motivation

A number of Bevy's features can't be used in a rollback context (with crates such as bevy_ggrs). This is either because they behave non-deterministically, don't implement reflect, rely on local system state, or are tied to the `Main` schedule.

## Roadmap

- [x] States
- [x] FrameCount
- [ ] Events
- [ ] Timers

## States

Bevy states when added through `app.add_state::<FooState>()` have two big problems:

1. They happen in the `StateTransition` schedule within the `MainSchedule`
2. If rolled back to the first frame, `OnEnter(InitialState)` is not re-run.

This crate provides an extension method, which adds workarounds to let you add schedules to the schedule you want, and a resource, `InitialStateEntered<S>` which can be rolled back and tracks whether the initial `OnEnter` should be run.

See the `states` example for usage with `bevy_ggrs`.

## Cargo features

- `bevy_ggrs`: Enable integration with `bevy_ggrs`

## Bevy Version Support

The `main` branch targets the latest bevy release.

I intend to support the `main` branch of Bevy in the `bevy-main` branch.

|bevy|bevy_roll|
|----|---------|
|none|main     |

## License

`bevy_roll` is dual-licensed under either

- MIT License (./LICENSE-MIT or http://opensource.org/licenses/MIT)
- Apache License, Version 2.0 (./LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0)

at your option.

## Contributions

PRs welcome!
