//! Fixed-storage cooperative kernel task scheduler.
//!
//! This is deliberately a scheduler core, not a context switcher yet. A task
//! entry is a short function that returns a [`TaskAction`] at a cooperative
//! yield point. The scheduler owns task state and wake-up deadlines; a later
//! architecture layer will give each task a stack and preserve CPU context
//! across yields.

use core::fmt;

pub mod context;
pub mod stack;

use context::TaskContext;
use stack::TaskStack;

pub const MAX_TASKS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskId(usize);

impl TaskId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping { until: u64 },
    Finished,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskAction {
    /// Return control to the scheduler and remain runnable.
    Yield,
    /// Do not run this task again until the given scheduler tick.
    SleepUntil(u64),
    /// Remove this task from future scheduling.
    Exit,
}

pub type TaskEntry = fn() -> TaskAction;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    NoTaskSlots,
    InvalidTask,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoTaskSlots => "no task slots available",
            Self::InvalidTask => "invalid task identifier",
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
    pub running: usize,
}

/// A bounded, allocation-free cooperative scheduler.
pub struct Scheduler {
    tasks: [Option<Task>; MAX_TASKS],
    stacks: [TaskStack; MAX_TASKS],
    contexts: [TaskContext; MAX_TASKS],
    next: usize,
}

impl Scheduler {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tasks: [None; MAX_TASKS],
            stacks: [TaskStack::new(); MAX_TASKS],
            contexts: [TaskContext::new(); MAX_TASKS],
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
        self.contexts[index].stack_pointer = self.stacks[index].top();
        Ok(TaskId(index))
    }

    #[must_use]
    pub fn state(&self, task: TaskId) -> Option<TaskState> {
        self.tasks.get(task.0)?.as_ref().map(|task| task.state)
    }

    /// Returns the saved CPU context for a task.
    ///
    /// The context is currently prepared but not switched to. A trampoline
    /// must be installed at the top of the stack before this is used by the
    /// scheduler's context-switch path.
    #[must_use]
    pub fn context(&self, task: TaskId) -> Option<&TaskContext> {
        self.tasks.get(task.0)?.as_ref()?;
        self.contexts.get(task.0)
    }

    /// Returns the stack owned by a task.
    #[must_use]
    pub fn stack(&self, task: TaskId) -> Option<&TaskStack> {
        self.tasks.get(task.0)?.as_ref()?;
        self.stacks.get(task.0)
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
        let task_id = TaskId(index);
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
            TaskAction::Exit => TaskState::Finished,
        };
        self.next = (index + 1) % MAX_TASKS;
        Ok(Some(task_id))
    }

    #[must_use]
    pub fn stats(&self) -> SchedulerStats {
        let mut stats = SchedulerStats::default();
        for task in self.tasks.iter().flatten() {
            stats.task_count += 1;
            match task.state {
                TaskState::Ready => stats.ready += 1,
                TaskState::Sleeping { .. } => stats.sleeping += 1,
                TaskState::Running => stats.running += 1,
                TaskState::Finished => {}
            }
        }
        stats
    }

    fn find_next_ready(&self) -> Option<usize> {
        (0..MAX_TASKS)
            .map(|offset| (self.next + offset) % MAX_TASKS)
            .find(|&index| self.tasks[index].is_some_and(|task| task.state == TaskState::Ready))
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
}
