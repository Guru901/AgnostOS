//! Fixed-storage cooperative kernel task scheduler.
//!
//! Tasks run on their own 4 KiB stacks and return to the scheduler through
//! [`context::switch`]. Dispatch is cooperative, single-core, and round-robin.
//! The kernel [`Mutex`] is not held while task code executes. Preemption and
//! multi-core scheduling remain future work.

use core::fmt;
use core::sync::atomic::Ordering;

pub mod context;
pub mod stack;

use crate::{globals::task::MAX_TASKS, kprintln};
use context::TaskContext;
use spin::Mutex;
use stack::TaskStack;

/// Kernel scheduler storage. Lives in `.bss` because the embedded task stacks
/// are far larger than the bootstrap stack window mapped in [`crate::paging`].
static SCHEDULER: Mutex<Scheduler> = Mutex::new(Scheduler::new());

#[cfg(not(test))]
static ACTIVE_PTR: core::sync::atomic::AtomicPtr<()> =
    core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

#[cfg(test)]
std::thread_local! {
    static ACTIVE_TLS: core::cell::Cell<*mut ()> =
        const { core::cell::Cell::new(core::ptr::null_mut()) };
}

fn set_active(scheduler: *mut Scheduler) {
    let ptr = scheduler.cast();
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
    #[cfg(test)]
    ACTIVE_TLS.with(|cell| cell.set(ptr));
    #[cfg(not(test))]
    ACTIVE_PTR.store(ptr, Ordering::Release);
}

fn active_scheduler() -> *mut Scheduler {
    let ptr = {
        #[cfg(test)]
        {
            ACTIVE_TLS.with(|cell| cell.get())
        }
        #[cfg(not(test))]
        {
            ACTIVE_PTR.load(Ordering::Acquire)
        }
    };
    ptr.cast()
}

/// Runs `f` with exclusive access to the kernel scheduler.
pub fn with_scheduler<F, R>(f: F) -> R
where
    F: FnOnce(&mut Scheduler) -> R,
{
    f(&mut SCHEDULER.lock())
}

/// Dispatches ready tasks on the kernel scheduler.
///
/// The scheduler lock is released while each task runs so a task may call
/// [`with_scheduler`] without deadlocking. Tasks must not call this function
/// or [`Scheduler::run_next`].
pub fn run_ready(now: u64, budget: usize) -> Result<usize, SchedulerError> {
    let mut dispatched = 0;
    while dispatched < budget {
        if with_scheduler(|scheduler| scheduler.begin_dispatch(now))?.is_none() {
            break;
        }
        // SAFETY: `begin_dispatch` armed the active scheduler pointer and
        // prepared a valid destination context. The kernel mutex is not held.
        unsafe {
            switch_to_active_task();
        }
        with_scheduler(|scheduler| scheduler.finish_dispatch(now))?;
        dispatched += 1;
    }
    Ok(dispatched)
}

/// Opaque task handle: slot index plus a generation that invalidates reaped IDs until it wraps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskId {
    index: usize,
    generation: u32,
}

impl TaskId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }
}

/// Lifecycle of a spawned slot. [`TaskAction`], [`Scheduler::wake_expired`], and explicit wake/cancel change it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    /// Waiting for the scheduler tick in `until`; [`Scheduler::wake_expired`] resumes it.
    Sleeping {
        until: u64,
    },
    /// Waiting for [`Scheduler::wake`]; the tick clock does not resume it.
    Blocked,
    Finished,
    Cancelled,
}

/// Cooperative yield result returned by a [`TaskEntry`] or [`yield_with`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskAction {
    /// Return control to the scheduler and remain runnable.
    Yield,
    /// Do not run this task again until the given scheduler tick.
    SleepUntil(u64),
    /// Block until another subsystem explicitly wakes this task.
    Block,
    /// Remove this task from future scheduling.
    Exit,
}

/// Function pointer used as a task body.
///
/// Returning a [`TaskAction`] suspends the trampoline; the next dispatch
/// resumes after that return and invokes the entry again. To keep locals
/// across a yield, call [`yield_with`] (or [`yield_now`]) instead of returning.
pub type TaskEntry = fn() -> TaskAction;

/// Initial `ret` target built by [`TaskStack::initialize_frame`].
///
/// Entered via `ret` from [`context::switch`] with 16-byte-aligned RSP, which
/// is not a SysV function entry. The `call` below pushes a return address so
/// [`task_start`] sees a legal ABI. [`task_start`] never returns; `ud2` is a
/// trap if it does.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn task_trampoline() {
    core::arch::naked_asm!(
        "call {start}",
        "ud2",
        start = sym task_start,
    );
}

#[cfg(not(target_arch = "x86_64"))]
pub(crate) extern "C" fn task_trampoline() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

extern "C" fn task_start() -> ! {
    loop {
        let entry = current_entry();
        let action = entry();
        yield_with(action);
    }
}

fn current_entry() -> TaskEntry {
    let scheduler = active_scheduler();
    assert!(
        !scheduler.is_null(),
        "task trampoline ran without an active scheduler"
    );
    // SAFETY: `begin_dispatch` stored this pointer, the scheduler is pinned
    // for the dispatch, and `current` names a live Running slot.
    unsafe {
        let index = (*scheduler)
            .current
            .expect("task trampoline requires a current task")
            .index;
        (*scheduler)
            .tasks
            .get(index)
            .and_then(|slot| slot.as_ref())
            .expect("current task slot is occupied")
            .entry
    }
}

/// Suspends the running task and switches to the scheduler.
///
/// When the task is dispatched again, this function returns and execution
/// continues on the task stack. Must be called from a running task.
pub fn yield_now() {
    yield_with(TaskAction::Yield);
}

/// Suspends the running task with `action`.
///
/// Resume continues after this call unless `action` is [`TaskAction::Exit`],
/// in which case the scheduler will not dispatch the task again.
pub fn yield_with(action: TaskAction) {
    let scheduler = active_scheduler();
    assert!(
        !scheduler.is_null(),
        "task::yield_with called outside a running task"
    );

    // SAFETY: the active pointer was armed for this dispatch; `current` is
    // the running task; both contexts live in the pinned scheduler.
    unsafe {
        (*scheduler).pending_action = Some(action);
        let index = (*scheduler)
            .current
            .expect("yield_with requires a current task")
            .index;
        switch_contexts(
            core::ptr::addr_of_mut!((*scheduler).contexts[index]),
            core::ptr::addr_of!((*scheduler).scheduler_context),
        );
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn switch_contexts(from: *mut TaskContext, to: *const TaskContext) {
    // SAFETY: caller guarantees live, non-aliased switch frames.
    unsafe {
        context::switch(from, to);
    }
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn switch_contexts(_from: *mut TaskContext, _to: *const TaskContext) {
    panic!("task switching requires x86_64");
}

unsafe fn switch_to_active_task() {
    let scheduler = active_scheduler();
    assert!(
        !scheduler.is_null(),
        "dispatch switch requires an active scheduler"
    );
    // SAFETY: `begin_dispatch` armed the pointer and set `current`.
    unsafe {
        let index = (*scheduler)
            .current
            .expect("dispatch switch requires a current task")
            .index;
        switch_contexts(
            core::ptr::addr_of_mut!((*scheduler).scheduler_context),
            core::ptr::addr_of!((*scheduler).contexts[index]),
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    NoTaskSlots,
    /// Index/generation does not match a live slot (including after [`Scheduler::reap`]).
    InvalidTask,
    TaskNotFinished,
    /// A task tried to dispatch while another task is already running.
    NestedDispatch,
    /// The `Scheduler` value moved after a task stack frame was built.
    Relocated,
    /// The task switched back without recording a [`TaskAction`].
    MissingAction,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoTaskSlots => "no task slots available",
            Self::InvalidTask => "invalid task identifier",
            Self::TaskNotFinished => "task has not finished",
            Self::NestedDispatch => "nested scheduler dispatch",
            Self::Relocated => "scheduler was moved",
            Self::MissingAction => "task returned without an action",
        })
    }
}

/// Occupied scheduler slot: entry point plus the last recorded [`TaskState`].
#[derive(Clone, Copy)]
struct Task {
    entry: TaskEntry,
    state: TaskState,
    started: bool,
}

/// Counts of tasks in each [`TaskState`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SchedulerStats {
    pub task_count: usize,
    pub ready: usize,
    pub sleeping: usize,
    pub blocked: usize,
    pub running: usize,
    pub finished: usize,
    pub cancelled: usize,
}

impl SchedulerStats {
    /// Prints live scheduler occupancy to the kernel console.
    pub fn taskinfo() {
        let stats = with_scheduler(|scheduler| scheduler.stats());
        kprintln!("task slots:  {} / {}", stats.task_count, MAX_TASKS);
        kprintln!("ready:       {}", stats.ready);
        kprintln!("sleeping:    {}", stats.sleeping);
        kprintln!("blocked:     {}", stats.blocked);
        kprintln!("running:     {}", stats.running);
        kprintln!("finished:    {}", stats.finished);
        kprintln!("cancelled:   {}", stats.cancelled);
    }
}

/// A bounded, allocation-free cooperative scheduler.
///
/// Task stacks live inside this value. After the first dispatch the
/// `Scheduler` must not be moved until those tasks are reaped; moving it
/// returns [`SchedulerError::Relocated`].
pub struct Scheduler {
    tasks: [Option<Task>; MAX_TASKS],
    stacks: [TaskStack; MAX_TASKS],
    contexts: [TaskContext; MAX_TASKS],
    generations: [u32; MAX_TASKS],
    current: Option<TaskId>,
    next: usize,
    scheduler_context: TaskContext,
    pending_action: Option<TaskAction>,
    pin_addr: usize,
}

impl Scheduler {
    /// Empty scheduler with every slot free. Safe to construct as a static.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tasks: [None; MAX_TASKS],
            stacks: [TaskStack::new(); MAX_TASKS],
            contexts: [TaskContext::new(); MAX_TASKS],
            generations: [0; MAX_TASKS],
            current: None,
            next: 0,
            scheduler_context: TaskContext::new(),
            pending_action: None,
            pin_addr: 0,
        }
    }

    fn location(&self) -> usize {
        self.stacks.as_ptr() as usize
    }

    /// Adds a task in the ready state. IDs are unique across later [`Self::reap`] of the same slot.
    pub fn spawn(&mut self, entry: TaskEntry) -> Result<TaskId, SchedulerError> {
        let Some((index, slot)) = self
            .tasks
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        else {
            return Err(SchedulerError::NoTaskSlots);
        };
        *slot = Some(Task {
            entry,
            state: TaskState::Ready,
            started: false,
        });
        self.generations[index] = self.generations[index].wrapping_add(1).max(1);
        Ok(TaskId {
            index,
            generation: self.generations[index],
        })
    }

    #[must_use]
    pub fn state(&self, task: TaskId) -> Option<TaskState> {
        Some(self.valid_task(task).ok()??.state)
    }

    #[must_use]
    pub const fn current(&self) -> Option<TaskId> {
        self.current
    }

    /// Wakes a blocked task. Sleeping tasks are woken by [`wake_expired`].
    pub fn wake(&mut self, task: TaskId) -> Result<(), SchedulerError> {
        let task_slot = self.valid_task(task)?.ok_or(SchedulerError::InvalidTask)?;
        if task_slot.state == TaskState::Blocked {
            let Some(task_slot) = self.tasks[task.index].as_mut() else {
                return Err(SchedulerError::InvalidTask);
            };
            task_slot.state = TaskState::Ready;
        }
        Ok(())
    }

    /// Cancels a task that has not already terminated.
    pub fn cancel(&mut self, task: TaskId) -> Result<(), SchedulerError> {
        let task_slot = self.valid_task(task)?.ok_or(SchedulerError::InvalidTask)?;
        if !matches!(task_slot.state, TaskState::Finished | TaskState::Cancelled) {
            let Some(task_slot) = self.tasks[task.index].as_mut() else {
                return Err(SchedulerError::InvalidTask);
            };
            task_slot.state = TaskState::Cancelled;
        }
        Ok(())
    }

    /// Reclaims a finished task slot so it can be reused by a later spawn.
    pub fn reap(&mut self, task: TaskId) -> Result<(), SchedulerError> {
        let slot = self.valid_task(task)?.ok_or(SchedulerError::InvalidTask)?;
        if !matches!(slot.state, TaskState::Finished | TaskState::Cancelled) {
            return Err(SchedulerError::TaskNotFinished);
        }
        self.tasks[task.index] = None;
        self.contexts[task.index] = TaskContext::new();
        Ok(())
    }

    /// Saved CPU context for a task.
    #[must_use]
    pub fn context(&self, task: TaskId) -> Option<&TaskContext> {
        self.valid_task(task).ok()??;
        self.contexts.get(task.index)
    }

    /// Returns the stack owned by a task.
    #[must_use]
    pub fn stack(&self, task: TaskId) -> Option<&TaskStack> {
        self.valid_task(task).ok()??;
        self.stacks.get(task.index)
    }

    /// Wakes all tasks whose sleep deadline has elapsed.
    pub fn wake_expired(&mut self, now: u64) {
        for task in self.tasks.iter_mut().flatten() {
            if let TaskState::Sleeping { until } = task.state
                && now >= until
            {
                task.state = TaskState::Ready;
            }
        }
    }

    /// Prepares one ready task for a context switch. Does not run the task.
    fn begin_dispatch(&mut self, now: u64) -> Result<Option<TaskId>, SchedulerError> {
        if !active_scheduler().is_null() {
            return Err(SchedulerError::NestedDispatch);
        }
        if self.pin_addr != 0 && self.pin_addr != self.location() {
            return Err(SchedulerError::Relocated);
        }
        self.wake_expired(now);
        let Some(index) = self.find_next_ready() else {
            return Ok(None);
        };
        let task_id = TaskId {
            index,
            generation: self.generations[index],
        };
        let started = self.tasks[index]
            .as_ref()
            .ok_or(SchedulerError::InvalidTask)?
            .started;
        if !started {
            let context = self.contexts[index];
            self.contexts[index].stack_pointer = self.stacks[index].initialize_frame(&context);
            self.pin_addr = self.location();
        }
        let task = self.tasks[index]
            .as_mut()
            .ok_or(SchedulerError::InvalidTask)?;
        task.started = true;
        task.state = TaskState::Running;
        self.current = Some(task_id);
        self.pending_action = None;
        set_active(self as *mut Scheduler);
        Ok(Some(task_id))
    }

    /// Applies the [`TaskAction`] recorded by the task that just switched back.
    fn finish_dispatch(&mut self, now: u64) -> Result<TaskId, SchedulerError> {
        let task_id = self.current.ok_or(SchedulerError::InvalidTask)?;
        let action = self
            .pending_action
            .take()
            .ok_or(SchedulerError::MissingAction)?;
        let task = self.tasks[task_id.index]
            .as_mut()
            .ok_or(SchedulerError::InvalidTask)?;
        task.state = match action {
            TaskAction::Yield => TaskState::Ready,
            TaskAction::SleepUntil(until) if until <= now => TaskState::Ready,
            TaskAction::SleepUntil(until) => TaskState::Sleeping { until },
            TaskAction::Block => TaskState::Blocked,
            TaskAction::Exit => TaskState::Finished,
        };
        self.current = None;
        self.next = (task_id.index + 1) % MAX_TASKS;
        set_active(core::ptr::null_mut());
        Ok(task_id)
    }

    /// Runs at most one ready task and returns its identifier.
    ///
    /// The task is marked running before it is switched in. The recorded
    /// [`TaskAction`] is the only way it leaves Running.
    pub fn run_next(&mut self, now: u64) -> Result<Option<TaskId>, SchedulerError> {
        let Some(task_id) = self.begin_dispatch(now)? else {
            return Ok(None);
        };
        // SAFETY: `begin_dispatch` armed the active scheduler and task frame.
        unsafe {
            switch_to_active_task();
        }
        self.finish_dispatch(now)?;
        Ok(Some(task_id))
    }

    /// Runs ready tasks until the scheduler is idle or `budget` dispatches
    /// have completed. A finite budget prevents a task that continually yields
    /// from starving the kernel's other work.
    pub fn run_ready(&mut self, now: u64, budget: usize) -> Result<usize, SchedulerError> {
        let mut dispatched = 0;
        while dispatched < budget {
            if self.run_next(now)?.is_none() {
                break;
            }
            dispatched += 1;
        }
        Ok(dispatched)
    }

    /// Occupancy counters for the live slots.
    #[must_use]
    pub fn stats(&self) -> SchedulerStats {
        let mut stats = SchedulerStats::default();
        for task in self.tasks.iter().flatten() {
            stats.task_count += 1;
            match task.state {
                TaskState::Ready => stats.ready += 1,
                TaskState::Sleeping { .. } => stats.sleeping += 1,
                TaskState::Blocked => stats.blocked += 1,
                TaskState::Running => stats.running += 1,
                TaskState::Finished => stats.finished += 1,
                TaskState::Cancelled => stats.cancelled += 1,
            }
        }
        stats
    }

    fn find_next_ready(&self) -> Option<usize> {
        (0..MAX_TASKS)
            .map(|offset| (self.next + offset) % MAX_TASKS)
            .find(|&index| self.tasks[index].is_some_and(|task| task.state == TaskState::Ready))
    }

    fn valid_task(&self, task: TaskId) -> Result<Option<&Task>, SchedulerError> {
        if task.index >= MAX_TASKS || self.generations[task.index] != task.generation {
            return Err(SchedulerError::InvalidTask);
        }
        Ok(self.tasks[task.index].as_ref())
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    static FIRST_RUNS: AtomicUsize = AtomicUsize::new(0);
    static SECOND_RUNS: AtomicUsize = AtomicUsize::new(0);
    static RESUME_LOG: AtomicU64 = AtomicU64::new(0);
    static NESTED_RESULT: Mutex<Option<SchedulerError>> = Mutex::new(None);

    fn first_task() -> TaskAction {
        FIRST_RUNS.fetch_add(1, Ordering::Relaxed);
        TaskAction::Yield
    }

    fn second_task() -> TaskAction {
        SECOND_RUNS.fetch_add(1, Ordering::Relaxed);
        TaskAction::Yield
    }

    #[test]
    fn schedules_ready_tasks_round_robin() {
        FIRST_RUNS.store(0, Ordering::Relaxed);
        SECOND_RUNS.store(0, Ordering::Relaxed);
        let mut scheduler = Scheduler::new();
        let first = scheduler.spawn(first_task).unwrap();
        let second = scheduler.spawn(second_task).unwrap();

        assert_eq!(scheduler.run_next(0).unwrap(), Some(first));
        assert_eq!(scheduler.run_next(0).unwrap(), Some(second));
        assert_eq!(scheduler.run_next(0).unwrap(), Some(first));
        assert_eq!(FIRST_RUNS.load(Ordering::Relaxed), 2);
        assert_eq!(SECOND_RUNS.load(Ordering::Relaxed), 1);
    }

    fn sleeping_task() -> TaskAction {
        TaskAction::SleepUntil(10)
    }

    #[test]
    fn sleeping_tasks_wake_at_their_deadline() {
        let mut scheduler = Scheduler::new();
        let task = scheduler.spawn(sleeping_task).unwrap();
        assert_eq!(scheduler.run_next(0).unwrap(), Some(task));
        assert_eq!(
            scheduler.state(task),
            Some(TaskState::Sleeping { until: 10 })
        );
        assert_eq!(scheduler.run_next(9).unwrap(), None);
        assert_eq!(scheduler.run_next(10).unwrap(), Some(task));
    }

    fn exiting_task() -> TaskAction {
        TaskAction::Exit
    }

    #[test]
    fn exiting_tasks_are_not_scheduled_again() {
        let mut scheduler = Scheduler::new();
        let task = scheduler.spawn(exiting_task).unwrap();
        assert_eq!(scheduler.run_next(0).unwrap(), Some(task));
        assert_eq!(scheduler.state(task), Some(TaskState::Finished));
        assert_eq!(scheduler.run_next(0).unwrap(), None);
        assert_eq!(scheduler.stats().task_count, 1);
    }

    fn blocking_task() -> TaskAction {
        TaskAction::Block
    }

    #[test]
    fn blocked_tasks_require_an_explicit_wake() {
        let mut scheduler = Scheduler::new();
        let task = scheduler.spawn(blocking_task).unwrap();
        assert_eq!(scheduler.run_next(0).unwrap(), Some(task));
        assert_eq!(scheduler.state(task), Some(TaskState::Blocked));
        assert_eq!(scheduler.run_next(0).unwrap(), None);
        scheduler.wake(task).unwrap();
        assert_eq!(scheduler.run_next(0).unwrap(), Some(task));
    }

    #[test]
    fn cancelled_tasks_can_be_reaped() {
        let mut scheduler = Scheduler::new();
        let task = scheduler.spawn(first_task).unwrap();
        scheduler.cancel(task).unwrap();
        assert_eq!(scheduler.state(task), Some(TaskState::Cancelled));
        scheduler.reap(task).unwrap();
        assert_eq!(scheduler.state(task), None);
    }

    #[test]
    fn dispatch_budget_prevents_a_yielding_task_from_monopolizing_the_loop() {
        let mut scheduler = Scheduler::new();
        scheduler.spawn(yielding_task).unwrap();
        assert_eq!(scheduler.run_ready(0, 3).unwrap(), 3);
        assert_eq!(scheduler.run_ready(0, 0).unwrap(), 0);
    }

    fn yielding_task() -> TaskAction {
        TaskAction::Yield
    }

    #[test]
    fn reaping_releases_a_slot_and_invalidates_the_old_id() {
        let mut scheduler = Scheduler::new();
        let old = scheduler.spawn(exiting_task).unwrap();
        scheduler.run_next(0).unwrap();
        scheduler.reap(old).unwrap();
        assert_eq!(scheduler.state(old), None);

        let replacement = scheduler.spawn(exiting_task).unwrap();
        assert_ne!(old, replacement);
        assert_eq!(
            scheduler.reap(replacement),
            Err(SchedulerError::TaskNotFinished)
        );
    }

    fn resume_task() -> TaskAction {
        let mut step = 1u64;
        RESUME_LOG.fetch_add(step, Ordering::Relaxed);
        yield_now();
        step += 1;
        RESUME_LOG.fetch_add(step, Ordering::Relaxed);
        TaskAction::Exit
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn yield_now_resumes_local_state_instead_of_restarting() {
        RESUME_LOG.store(0, Ordering::Relaxed);
        let mut scheduler = Scheduler::new();
        let task = scheduler.spawn(resume_task).unwrap();

        assert_eq!(scheduler.run_next(0).unwrap(), Some(task));
        assert_eq!(RESUME_LOG.load(Ordering::Relaxed), 1);
        assert_eq!(scheduler.state(task), Some(TaskState::Ready));
        assert_ne!(scheduler.context(task).unwrap().stack_pointer, 0);

        assert_eq!(scheduler.run_next(0).unwrap(), Some(task));
        assert_eq!(RESUME_LOG.load(Ordering::Relaxed), 1 + 2);
        assert_eq!(scheduler.state(task), Some(TaskState::Finished));
        assert_eq!(scheduler.run_next(0).unwrap(), None);
    }

    fn resume_a() -> TaskAction {
        let mut n = 0u64;
        n += 1;
        RESUME_A.store(n, Ordering::Relaxed);
        yield_now();
        n += 1;
        RESUME_A.store(n, Ordering::Relaxed);
        TaskAction::Exit
    }

    fn resume_b() -> TaskAction {
        let mut n = 0u64;
        n += 1;
        RESUME_B.store(n, Ordering::Relaxed);
        yield_now();
        n += 1;
        RESUME_B.store(n, Ordering::Relaxed);
        TaskAction::Exit
    }

    static RESUME_A: AtomicU64 = AtomicU64::new(0);
    static RESUME_B: AtomicU64 = AtomicU64::new(0);

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn round_robin_resumes_each_task_after_its_yield_point() {
        RESUME_A.store(0, Ordering::Relaxed);
        RESUME_B.store(0, Ordering::Relaxed);
        let mut scheduler = Scheduler::new();
        let a = scheduler.spawn(resume_a).unwrap();
        let b = scheduler.spawn(resume_b).unwrap();

        assert_eq!(scheduler.run_next(0).unwrap(), Some(a));
        assert_eq!(RESUME_A.load(Ordering::Relaxed), 1);
        assert_eq!(RESUME_B.load(Ordering::Relaxed), 0);

        assert_eq!(scheduler.run_next(0).unwrap(), Some(b));
        assert_eq!(RESUME_A.load(Ordering::Relaxed), 1);
        assert_eq!(RESUME_B.load(Ordering::Relaxed), 1);

        assert_eq!(scheduler.run_next(0).unwrap(), Some(a));
        assert_eq!(RESUME_A.load(Ordering::Relaxed), 2);
        assert_eq!(RESUME_B.load(Ordering::Relaxed), 1);

        assert_eq!(scheduler.run_next(0).unwrap(), Some(b));
        assert_eq!(RESUME_A.load(Ordering::Relaxed), 2);
        assert_eq!(RESUME_B.load(Ordering::Relaxed), 2);
    }

    fn nested_dispatch_task() -> TaskAction {
        *NESTED_RESULT.lock() = with_scheduler(|scheduler| scheduler.run_next(0).err());
        TaskAction::Exit
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn nested_dispatch_from_a_running_task_is_rejected() {
        *NESTED_RESULT.lock() = None;
        let mut scheduler = Scheduler::new();
        scheduler.spawn(nested_dispatch_task).unwrap();
        scheduler.run_next(0).unwrap();
        assert_eq!(*NESTED_RESULT.lock(), Some(SchedulerError::NestedDispatch));
    }

    #[test]
    fn moving_the_scheduler_after_a_task_starts_is_detected() {
        let mut scheduler = Scheduler::new();
        scheduler.spawn(yielding_task).unwrap();
        scheduler.run_next(0).unwrap();
        let mut moved = scheduler;
        assert_eq!(moved.run_next(0), Err(SchedulerError::Relocated));
    }

    #[test]
    fn wake_does_not_resume_sleeping_tasks() {
        let mut scheduler = Scheduler::new();
        let task = scheduler.spawn(sleeping_task).unwrap();
        scheduler.run_next(0).unwrap();
        scheduler.wake(task).unwrap();
        assert_eq!(
            scheduler.state(task),
            Some(TaskState::Sleeping { until: 10 })
        );
        assert_eq!(scheduler.run_next(0).unwrap(), None);
    }
}
