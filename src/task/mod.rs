//! Fixed-storage cooperative kernel task scheduler.
//!
//! This is a fixed-storage cooperative scheduler with architecture context
//! preparation. A task entry is a short function that returns a [`TaskAction`]
//! at a cooperative yield point. The scheduler owns task state, stacks, and
//! wake-up deadlines; preemption and multi-core scheduling remain future work.

use core::fmt;

pub mod context;
pub mod stack;

use context::TaskContext;
use stack::TaskStack;

pub const MAX_TASKS: usize = 64;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping { until: u64 },
    Blocked,
    Finished,
    Cancelled,
}

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

pub type TaskEntry = fn() -> TaskAction;

/// Initial return target for a task context.
pub(crate) extern "C" fn task_trampoline() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    NoTaskSlots,
    InvalidTask,
    TaskNotFinished,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoTaskSlots => "no task slots available",
            Self::InvalidTask => "invalid task identifier",
            Self::TaskNotFinished => "task has not finished",
        })
    }
}

#[derive(Clone, Copy)]
struct Task {
    entry: TaskEntry,
    state: TaskState,
}

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

/// A bounded, allocation-free cooperative scheduler.
pub struct Scheduler {
    tasks: [Option<Task>; MAX_TASKS],
    stacks: [TaskStack; MAX_TASKS],
    contexts: [TaskContext; MAX_TASKS],
    generations: [u32; MAX_TASKS],
    current: Option<TaskId>,
    next: usize,
}

impl Scheduler {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tasks: [None; MAX_TASKS],
            stacks: [TaskStack::new(); MAX_TASKS],
            contexts: [TaskContext::new(); MAX_TASKS],
            generations: [0; MAX_TASKS],
            current: None,
            next: 0,
        }
    }

    /// Adds a task in the ready state.
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

    /// Returns the saved CPU context for a task.
    ///
    /// Returns the saved CPU context for a task. Its stack frame is prepared
    /// immediately before the task is executed, after the scheduler's final
    /// location is known.
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

    /// Runs at most one ready task and returns its identifier.
    ///
    /// The task is marked running before its entry is called. Its returned
    /// action is the only supported way to transition it back to ready,
    /// sleeping, or finished; this keeps task lifetime explicit before real
    /// register-context ownership is introduced.
    pub fn run_next(&mut self, now: u64) -> Result<Option<TaskId>, SchedulerError> {
        self.wake_expired(now);
        let Some(index) = self.find_next_ready() else {
            return Ok(None);
        };
        let task_id = TaskId {
            index,
            generation: self.generations[index],
        };
        self.contexts[index].stack_pointer =
            self.stacks[index].initialize_frame(&self.contexts[index]);
        self.current = Some(task_id);
        let entry = {
            let task = self.tasks[index]
                .as_mut()
                .ok_or(SchedulerError::InvalidTask)?;
            task.state = TaskState::Running;
            task.entry
        };

        let action = entry();
        let task = self.tasks[index]
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
        self.next = (index + 1) % MAX_TASKS;
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
    use core::sync::atomic::{AtomicUsize, Ordering};

    static FIRST_RUNS: AtomicUsize = AtomicUsize::new(0);
    static SECOND_RUNS: AtomicUsize = AtomicUsize::new(0);

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
}
