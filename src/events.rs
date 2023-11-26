//! This is a fork of `bevy::ecs::event`, which implements and requires `Clone`
//! in all the appropriate places, so events can easily be rolled back.

use bevy::{
    ecs::system::SystemParam,
    prelude::*,
    utils::{self as bevy_utils, detailed_trace},
};
use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    iter::Chain,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    slice::Iter,
};

/// A type that can be stored in an [`RollEvents<E>`] resource
/// You can conveniently access events using the [`RollEventReader`] and [`RollEventWriter`] system parameter.
///
/// Events must be thread-safe.
pub trait RollEvent: Event + Clone {}

impl<T: Event + Clone> RollEvent for T {}

/// An `EventId` uniquely identifies an event stored in a specific [`World`].
///
/// An `EventId` can among other things be used to trace the flow of an event from the point it was
/// sent to the point it was processed.
///
/// [`World`]: crate::world::World
pub struct RollEventId<E: RollEvent> {
    /// Uniquely identifies the event associated with this ID.
    // This value corresponds to the order in which each event was added to the world.
    pub id: usize,
    _marker: PhantomData<E>,
}

impl<E: RollEvent> Copy for RollEventId<E> {}
impl<E: RollEvent> Clone for RollEventId<E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: RollEvent> fmt::Display for RollEventId<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

impl<E: RollEvent> fmt::Debug for RollEventId<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "event<{}>#{}",
            std::any::type_name::<E>().split("::").last().unwrap(),
            self.id,
        )
    }
}

impl<E: RollEvent> PartialEq for RollEventId<E> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<E: RollEvent> Eq for RollEventId<E> {}

impl<E: RollEvent> PartialOrd for RollEventId<E> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<E: RollEvent> Ord for RollEventId<E> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.id.cmp(&other.id)
    }
}

impl<E: RollEvent> Hash for RollEventId<E> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Hash::hash(&self.id, state);
    }
}

#[derive(Debug, Clone)]
struct RollEventInstance<E: RollEvent> {
    pub event_id: RollEventId<E>,
    pub event: E,
}

/// An event collection that represents the events that occurred within the last two
/// [`RollEvents::update`] calls.
/// Events can be written to using an [`RollEventWriter`]
/// and are typically cheaply read using an [`RollEventReader`].
///
/// Each event can be consumed by multiple systems, in parallel,
/// with consumption tracked by the [`RollEventReader`] on a per-system basis.
///
/// If no [ordering](https://github.com/bevyengine/bevy/blob/main/examples/ecs/ecs_guide.rs)
/// is applied between writing and reading systems, there is a risk of a race condition.
/// This means that whether the events arrive before or after the next [`RollEvents::update`] is unpredictable.
///
/// This collection is meant to be paired with a system that calls
/// [`RollEvents::update`] exactly once per update/frame.
///
/// [`roll_event_update_system`] is a system that does this, typically initialized automatically using
/// [`add_event`](https://docs.rs/bevy/*/bevy/app/struct.App.html#method.add_event).
/// [`RollEventReader`]s are expected to read events from this collection at least once per loop/frame.
/// Events will persist across a single frame boundary and so ordering of event producers and
/// consumers is not critical (although poorly-planned ordering may cause accumulating lag).
/// If events are not handled by the end of the frame after they are updated, they will be
/// dropped silently.
///
/// # Example
/// ```
/// use bevy_roll_safe::event::{RollEvent, RollEvents};
///
/// #[derive(Event, Clone)]
/// struct MyEvent {
///     value: usize
/// }
///
/// // setup
/// let mut events = RollEvents::<MyEvent>::default();
/// let mut reader = events.get_reader();
///
/// // run this once per update/frame
/// events.update();
///
/// // somewhere else: send an event
/// events.send(MyEvent { value: 1 });
///
/// // somewhere else: read the events
/// for event in reader.iter(&events) {
///     assert_eq!(event.value, 1)
/// }
///
/// // events are only processed once per reader
/// assert_eq!(reader.iter(&events).count(), 0);
/// ```
///
/// # Details
///
/// [`RollEvents`] is implemented using a variation of a double buffer strategy.
/// Each call to [`update`](Events::update) swaps buffers and clears out the oldest one.
/// - [`RollEventReader`]s will read events from both buffers.
/// - [`RollEventReader`]s that read at least once per update will never drop events.
/// - [`RollEventReader`]s that read once within two updates might still receive some events
/// - [`RollEventReader`]s that read after two updates are guaranteed to drop all events that occurred
/// before those updates.
///
/// The buffers in [`RollEvents`] will grow indefinitely if [`update`](RollEvents::update) is never called.
///
/// An alternative call pattern would be to call [`update`](RollEvents::update)
/// manually across frames to control when events are cleared.
/// This complicates consumption and risks ever-expanding memory usage if not cleaned up,
/// but can be done by adding your event as a resource instead of using
/// [`add_event`](https://docs.rs/bevy/*/bevy/app/struct.App.html#method.add_event).
///
/// [Example usage.](https://github.com/bevyengine/bevy/blob/latest/examples/ecs/event.rs)
/// [Example usage standalone.](https://github.com/bevyengine/bevy/blob/latest/crates/bevy_ecs/examples/events.rs)
///
#[derive(Debug, Resource, Clone)]
pub struct RollEvents<E: RollEvent> {
    /// Holds the oldest still active events.
    /// Note that a.start_event_count + a.len() should always === events_b.start_event_count.
    events_a: RollEventSequence<E>,
    /// Holds the newer events.
    events_b: RollEventSequence<E>,
    event_count: usize,
}

// Derived Default impl would incorrectly require E: Default
impl<E: RollEvent> Default for RollEvents<E> {
    fn default() -> Self {
        Self {
            events_a: Default::default(),
            events_b: Default::default(),
            event_count: Default::default(),
        }
    }
}

impl<E: RollEvent> RollEvents<E> {
    /// Returns the index of the oldest event stored in the event buffer.
    pub fn oldest_event_count(&self) -> usize {
        self.events_a
            .start_event_count
            .min(self.events_b.start_event_count)
    }

    /// "Sends" an `event` by writing it to the current event buffer. [`RollEventReader`]s can then read
    /// the event.
    pub fn send(&mut self, event: E) {
        let event_id = RollEventId {
            id: self.event_count,
            _marker: PhantomData,
        };
        detailed_trace!("Events::send() -> id: {}", event_id);

        let event_instance = RollEventInstance { event_id, event };

        self.events_b.push(event_instance);
        self.event_count += 1;
    }

    /// Sends the default value of the event. Useful when the event is an empty struct.
    pub fn send_default(&mut self)
    where
        E: Default,
    {
        self.send(Default::default());
    }

    /// Gets a new [`ManualEventReader`]. This will include all events already in the event buffers.
    pub fn get_reader(&self) -> ManualEventReader<E> {
        ManualEventReader::default()
    }

    /// Gets a new [`ManualEventReader`]. This will ignore all events already in the event buffers.
    /// It will read all future events.
    pub fn get_reader_current(&self) -> ManualEventReader<E> {
        ManualEventReader {
            last_event_count: self.event_count,
            ..Default::default()
        }
    }

    /// Swaps the event buffers and clears the oldest event buffer. In general, this should be
    /// called once per frame/update.
    ///
    /// If you need access to the events that were removed, consider using [`Events::update_drain`].
    pub fn update(&mut self) {
        let _ = self.update_drain();
    }

    /// Swaps the event buffers and drains the oldest event buffer, returning an iterator
    /// of all events that were removed. In general, this should be called once per frame/update.
    ///
    /// If you do not need to take ownership of the removed events, use [`Events::update`] instead.
    #[must_use = "If you do not need the returned events, call .update() instead."]
    pub fn update_drain(&mut self) -> impl Iterator<Item = E> + '_ {
        std::mem::swap(&mut self.events_a, &mut self.events_b);
        let iter = self.events_b.events.drain(..);
        self.events_b.start_event_count = self.event_count;
        debug_assert_eq!(
            self.events_a.start_event_count + self.events_a.len(),
            self.events_b.start_event_count
        );

        iter.map(|e| e.event)
    }

    #[inline]
    fn reset_start_event_count(&mut self) {
        self.events_a.start_event_count = self.event_count;
        self.events_b.start_event_count = self.event_count;
    }

    /// Removes all events.
    #[inline]
    pub fn clear(&mut self) {
        self.reset_start_event_count();
        self.events_a.clear();
        self.events_b.clear();
    }

    /// Returns the number of events currently stored in the event buffer.
    #[inline]
    pub fn len(&self) -> usize {
        self.events_a.len() + self.events_b.len()
    }

    /// Returns true if there are no events currently stored in the event buffer.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Creates a draining iterator that removes all events.
    pub fn drain(&mut self) -> impl Iterator<Item = E> + '_ {
        self.reset_start_event_count();

        // Drain the oldest events first, then the newest
        self.events_a
            .drain(..)
            .chain(self.events_b.drain(..))
            .map(|i| i.event)
    }

    /// Iterates over events that happened since the last "update" call.
    /// WARNING: You probably don't want to use this call. In most cases you should use an
    /// [`RollEventReader`]. You should only use this if you know you only need to consume events
    /// between the last `update()` call and your call to `iter_current_update_events`.
    /// If events happen outside that window, they will not be handled. For example, any events that
    /// happen after this call and before the next `update()` call will be dropped.
    pub fn iter_current_update_events(&self) -> impl ExactSizeIterator<Item = &E> {
        self.events_b.iter().map(|i| &i.event)
    }

    /// Get a specific event by id if it still exists in the events buffer.
    pub fn get_event(&self, id: usize) -> Option<(&E, RollEventId<E>)> {
        if id < self.oldest_id() {
            return None;
        }

        let sequence = self.sequence(id);
        let index = id.saturating_sub(sequence.start_event_count);

        sequence
            .get(index)
            .map(|instance| (&instance.event, instance.event_id))
    }

    /// Oldest id still in the events buffer.
    pub fn oldest_id(&self) -> usize {
        self.events_a.start_event_count
    }

    /// Which event buffer is this event id a part of.
    fn sequence(&self, id: usize) -> &RollEventSequence<E> {
        if id < self.events_b.start_event_count {
            &self.events_a
        } else {
            &self.events_b
        }
    }
}

impl<E: RollEvent> std::iter::Extend<E> for RollEvents<E> {
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = E>,
    {
        let old_count = self.event_count;
        let mut event_count = self.event_count;
        let events = iter.into_iter().map(|event| {
            let event_id = RollEventId {
                id: event_count,
                _marker: PhantomData,
            };
            event_count += 1;
            RollEventInstance { event_id, event }
        });

        self.events_b.extend(events);

        if old_count != event_count {
            detailed_trace!(
                "Events::extend() -> ids: ({}..{})",
                self.event_count,
                event_count
            );
        }

        self.event_count = event_count;
    }
}

#[derive(Debug, Clone)]
struct RollEventSequence<E: RollEvent> {
    events: Vec<RollEventInstance<E>>,
    start_event_count: usize,
}

// Derived Default impl would incorrectly require E: Default
impl<E: RollEvent> Default for RollEventSequence<E> {
    fn default() -> Self {
        Self {
            events: Default::default(),
            start_event_count: Default::default(),
        }
    }
}

impl<E: RollEvent> Deref for RollEventSequence<E> {
    type Target = Vec<RollEventInstance<E>>;

    fn deref(&self) -> &Self::Target {
        &self.events
    }
}

impl<E: RollEvent> DerefMut for RollEventSequence<E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.events
    }
}

/// Reads events of type `T` in order and tracks which events have already been read.
#[derive(SystemParam, Debug)]
pub struct RollEventReader<'w, 's, E: RollEvent> {
    reader: Local<'s, ManualEventReader<E>>,
    events: Res<'w, RollEvents<E>>,
}

impl<'w, 's, E: RollEvent> RollEventReader<'w, 's, E> {
    /// Iterates over the events this [`RollEventReader`] has not seen yet. This updates the
    /// [`RollEventReader`]'s event counter, which means subsequent event reads will not include events
    /// that happened before now.
    pub fn read(&mut self) -> RollEventIterator<'_, E> {
        self.reader.read(&self.events)
    }

    /// Iterates over the events this [`RollEventReader`] has not seen yet. This updates the
    /// [`RollEventReader`]'s event counter, which means subsequent event reads will not include events
    /// that happened before now.
    #[deprecated = "use `.read()` instead."]
    pub fn iter(&mut self) -> RollEventIterator<'_, E> {
        self.reader.read(&self.events)
    }

    /// Like [`read`](Self::read), except also returning the [`EventId`] of the events.
    pub fn read_with_id(&mut self) -> EventIteratorWithId<'_, E> {
        self.reader.read_with_id(&self.events)
    }

    /// Like [`iter`](Self::iter), except also returning the [`EventId`] of the events.
    #[deprecated = "use `.read_with_id() instead."]
    pub fn iter_with_id(&mut self) -> EventIteratorWithId<'_, E> {
        self.reader.read_with_id(&self.events)
    }

    /// Determines the number of events available to be read from this [`RollEventReader`] without consuming any.
    pub fn len(&self) -> usize {
        self.reader.len(&self.events)
    }

    /// Returns `true` if there are no events available to read.
    ///
    /// # Example
    ///
    /// The following example shows a useful pattern where some behavior is triggered if new events are available.
    /// [`RollEventReader::clear()`] is used so the same events don't re-trigger the behavior the next time the system runs.
    ///
    /// ```
    /// # use bevy_ecs::prelude::*;
    /// #
    /// #[derive(Event)]
    /// struct CollisionEvent;
    ///
    /// fn play_collision_sound(mut events: RollEventReader<CollisionEvent>) {
    ///     if !events.is_empty() {
    ///         events.clear();
    ///         // Play a sound
    ///     }
    /// }
    /// # bevy_ecs::system::assert_is_system(play_collision_sound);
    /// ```
    pub fn is_empty(&self) -> bool {
        self.reader.is_empty(&self.events)
    }

    /// Consumes all available events.
    ///
    /// This means these events will not appear in calls to [`RollEventReader::iter()`] or
    /// [`RollEventReader::iter_with_id()`] and [`RollEventReader::is_empty()`] will return `true`.
    ///
    /// For usage, see [`RollEventReader::is_empty()`].
    pub fn clear(&mut self) {
        self.reader.clear(&self.events);
    }
}

/// Sends events of type `T`.
///
/// # Usage
///
/// `RollEventWriter`s are usually declared as a [`SystemParam`].
/// ```
/// # use bevy_ecs::prelude::*;
///
/// #[derive(Event)]
/// pub struct MyEvent; // Custom event type.
/// fn my_system(mut writer: RollEventWriter<MyEvent>) {
///     writer.send(MyEvent);
/// }
///
/// # bevy_ecs::system::assert_is_system(my_system);
/// ```
///
/// # Limitations
///
/// `RollEventWriter` can only send events of one specific type, which must be known at compile-time.
/// This is not a problem most of the time, but you may find a situation where you cannot know
/// ahead of time every kind of event you'll need to send. In this case, you can use the "type-erased event" pattern.
///
/// ```
/// # use bevy_ecs::{prelude::*, event::Events};
/// # #[derive(Event)]
/// # pub struct MyEvent;
/// fn send_untyped(mut commands: Commands) {
///     // Send an event of a specific type without having to declare that
///     // type as a SystemParam.
///     //
///     // Effectively, we're just moving the type parameter from the /type/ to the /method/,
///     // which allows one to do all kinds of clever things with type erasure, such as sending
///     // custom events to unknown 3rd party plugins (modding API).
///     //
///     // NOTE: the event won't actually be sent until commands get applied during
///     // apply_deferred.
///     commands.add(|w: &mut World| {
///         w.send_event(MyEvent);
///     });
/// }
/// ```
/// Note that this is considered *non-idiomatic*, and should only be used when `RollEventWriter` will not work.
#[derive(SystemParam)]
pub struct RollEventWriter<'w, E: RollEvent> {
    events: ResMut<'w, RollEvents<E>>,
}

impl<'w, E: RollEvent> RollEventWriter<'w, E> {
    /// Sends an `event`, which can later be read by [`RollEventReader`]s.
    ///
    /// See [`Events`] for details.
    pub fn send(&mut self, event: E) {
        self.events.send(event);
    }

    /// Sends a list of `events` all at once, which can later be read by [`RollEventReader`]s.
    /// This is more efficient than sending each event individually.
    ///
    /// See [`Events`] for details.
    pub fn send_batch(&mut self, events: impl IntoIterator<Item = E>) {
        self.events.extend(events);
    }

    /// Sends the default value of the event. Useful when the event is an empty struct.
    pub fn send_default(&mut self)
    where
        E: Default,
    {
        self.events.send_default();
    }
}

/// Stores the state for an [`RollEventReader`].
/// Access to the [`Events<E>`] resource is required to read any incoming events.
#[derive(Debug)]
pub struct ManualEventReader<E: RollEvent> {
    last_event_count: usize,
    _marker: PhantomData<E>,
}

impl<E: RollEvent> Default for ManualEventReader<E> {
    fn default() -> Self {
        ManualEventReader {
            last_event_count: 0,
            _marker: Default::default(),
        }
    }
}

#[allow(clippy::len_without_is_empty)] // Check fails since the is_empty implementation has a signature other than `(&self) -> bool`
impl<E: RollEvent> ManualEventReader<E> {
    /// See [`RollEventReader::read`]
    pub fn read<'a>(&'a mut self, events: &'a RollEvents<E>) -> RollEventIterator<'a, E> {
        self.read_with_id(events).without_id()
    }

    /// See [`RollEventReader::iter`]
    #[deprecated = "use `.read()` instead."]
    pub fn iter<'a>(&'a mut self, events: &'a RollEvents<E>) -> RollEventIterator<'a, E> {
        self.read_with_id(events).without_id()
    }

    /// See [`RollEventReader::read_with_id`]
    pub fn read_with_id<'a>(&'a mut self, events: &'a RollEvents<E>) -> EventIteratorWithId<'a, E> {
        EventIteratorWithId::new(self, events)
    }

    /// See [`RollEventReader::iter_with_id`]
    #[deprecated = "use `.read_with_id() instead."]
    pub fn iter_with_id<'a>(&'a mut self, events: &'a RollEvents<E>) -> EventIteratorWithId<'a, E> {
        EventIteratorWithId::new(self, events)
    }

    /// See [`RollEventReader::len`]
    pub fn len(&self, events: &RollEvents<E>) -> usize {
        // The number of events in this reader is the difference between the most recent event
        // and the last event seen by it. This will be at most the number of events contained
        // with the events (any others have already been dropped)
        // TODO: Warn when there are dropped events, or return e.g. a `Result<usize, (usize, usize)>`
        events
            .event_count
            .saturating_sub(self.last_event_count)
            .min(events.len())
    }

    /// Amount of events we missed.
    pub fn missed_events(&self, events: &RollEvents<E>) -> usize {
        events
            .oldest_event_count()
            .saturating_sub(self.last_event_count)
    }

    /// See [`RollEventReader::is_empty()`]
    pub fn is_empty(&self, events: &RollEvents<E>) -> bool {
        self.len(events) == 0
    }

    /// See [`RollEventReader::clear()`]
    pub fn clear(&mut self, events: &RollEvents<E>) {
        self.last_event_count = events.event_count;
    }
}

/// An iterator that yields any unread events from an [`RollEventReader`] or [`ManualEventReader`].
#[derive(Debug)]
pub struct RollEventIterator<'a, E: RollEvent> {
    iter: EventIteratorWithId<'a, E>,
}

/// An iterator that yields any unread events from an [`RollEventReader`] or [`ManualEventReader`].
///
/// This is a type alias for [`EventIterator`], which used to be called `ManualEventIterator`.
/// This type alias will be removed in the next release of bevy, so you should use [`EventIterator`] directly instead.
#[deprecated = "This type has been renamed to `EventIterator`."]
pub type ManualEventIterator<'a, E> = RollEventIterator<'a, E>;

impl<'a, E: RollEvent> Iterator for RollEventIterator<'a, E> {
    type Item = &'a E;
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|(event, _)| event)
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.iter.nth(n).map(|(event, _)| event)
    }

    fn last(self) -> Option<Self::Item>
    where
        Self: Sized,
    {
        self.iter.last().map(|(event, _)| event)
    }

    fn count(self) -> usize {
        self.iter.count()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, E: RollEvent> ExactSizeIterator for RollEventIterator<'a, E> {
    fn len(&self) -> usize {
        self.iter.len()
    }
}

/// An iterator that yields any unread events (and their IDs) from an [`RollEventReader`] or [`ManualEventReader`].
#[derive(Debug)]
pub struct EventIteratorWithId<'a, E: RollEvent> {
    reader: &'a mut ManualEventReader<E>,
    chain: Chain<Iter<'a, RollEventInstance<E>>, Iter<'a, RollEventInstance<E>>>,
    unread: usize,
}

/// An iterator that yields any unread events (and their IDs) from an [`RollEventReader`] or [`ManualEventReader`].
///
/// This is a type alias for [`EventIteratorWithId`], which used to be called `ManualEventIteratorWithId`.
/// This type alias will be removed in the next release of bevy, so you should use [`EventIteratorWithId`] directly instead.
#[deprecated = "This type has been renamed to `EventIteratorWithId`."]
pub type ManualEventIteratorWithId<'a, E> = EventIteratorWithId<'a, E>;

impl<'a, E: RollEvent> EventIteratorWithId<'a, E> {
    /// Creates a new iterator that yields any `events` that have not yet been seen by `reader`.
    pub fn new(reader: &'a mut ManualEventReader<E>, events: &'a RollEvents<E>) -> Self {
        let a_index = (reader.last_event_count).saturating_sub(events.events_a.start_event_count);
        let b_index = (reader.last_event_count).saturating_sub(events.events_b.start_event_count);
        let a = events.events_a.get(a_index..).unwrap_or_default();
        let b = events.events_b.get(b_index..).unwrap_or_default();

        let unread_count = a.len() + b.len();
        // Ensure `len` is implemented correctly
        debug_assert_eq!(unread_count, reader.len(events));
        reader.last_event_count = events.event_count - unread_count;
        // Iterate the oldest first, then the newer events
        let chain = a.iter().chain(b.iter());

        Self {
            reader,
            chain,
            unread: unread_count,
        }
    }

    /// Iterate over only the events.
    pub fn without_id(self) -> RollEventIterator<'a, E> {
        RollEventIterator { iter: self }
    }
}

impl<'a, E: RollEvent> Iterator for EventIteratorWithId<'a, E> {
    type Item = (&'a E, RollEventId<E>);
    fn next(&mut self) -> Option<Self::Item> {
        match self
            .chain
            .next()
            .map(|instance| (&instance.event, instance.event_id))
        {
            Some(item) => {
                detailed_trace!("RollEventReader::iter() -> {}", item.1);
                self.reader.last_event_count += 1;
                self.unread -= 1;
                Some(item)
            }
            None => None,
        }
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        if let Some(RollEventInstance { event_id, event }) = self.chain.nth(n) {
            self.reader.last_event_count += n + 1;
            self.unread -= n + 1;
            Some((event, *event_id))
        } else {
            self.reader.last_event_count += self.unread;
            self.unread = 0;
            None
        }
    }

    fn last(self) -> Option<Self::Item>
    where
        Self: Sized,
    {
        let RollEventInstance { event_id, event } = self.chain.last()?;
        self.reader.last_event_count += self.unread;
        Some((event, *event_id))
    }

    fn count(self) -> usize {
        self.reader.last_event_count += self.unread;
        self.unread
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chain.size_hint()
    }
}

impl<'a, E: RollEvent> ExactSizeIterator for EventIteratorWithId<'a, E> {
    fn len(&self) -> usize {
        self.unread
    }
}

/// A system that calls [`RollEvents::update`] once per frame.
pub fn roll_event_update_system<T: RollEvent>(mut events: ResMut<RollEvents<T>>) {
    events.update();
}

/// A run condition that checks if the event's [`roll_event_update_system`]
/// needs to run or not.
pub fn roll_event_update_condition<T: RollEvent>(events: Res<RollEvents<T>>) -> bool {
    !events.events_a.is_empty() || !events.events_b.is_empty()
}
