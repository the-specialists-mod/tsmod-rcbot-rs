//! The bot's task queue (`bot.h` `CBotTask`, `CBotTasks`; `bot.cpp` `CBotTask::operator==`,
//! `CBot::AddTask`, `CBot::AddPriorityTask`).
//!
//! Upstream keeps tasks in a `dataQueue`: the current task is the front, `AddTaskToEnd` appends,
//! `AddTaskToFront` first removes an equal task (and the rest of its schedule) and then pushes to
//! the front. A schedule is the set of tasks sharing a schedule id: failing one task fails them
//! all. Only the tasks a TS bot uses are here (design §2).

use std::collections::VecDeque;

use crate::Vector;

/// An entity a task refers to: upstream's `MyEHandle` (the edict and its serial number, so a
/// reused slot doesn't count as the same entity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntRef {
    /// The entity index.
    pub index: u32,
    /// `edict_t::serialnumber` when the handle was taken.
    pub serial: i32,
}

/// `eBotTask`: the tasks the TS bot uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    /// `BOT_TASK_NONE`.
    None,
    /// `BOT_TASK_FIND_PATH`: int = goal waypoint (-1 random), float = mode (-1 random goal, -2
    /// "find the path and finish", -3 path search pending, 0 go to the vector), edict/vector =
    /// where to.
    FindPath,
    /// `BOT_TASK_NORMAL_ATTACK`: edict = the enemy, float = shoot until this time (0: no limit).
    NormalAttack,
    /// `BOT_TASK_SEARCH_FOR_ENEMY`: float = how long.
    SearchForEnemy,
    /// `BOT_TASK_RELOAD`.
    Reload,
    /// `BOT_TASK_WAIT_AND_FACE_VECTOR`: float = how long, vector = where to look.
    WaitAndFaceVector,
    /// `BOT_TASK_FACE_VECTOR`.
    FaceVector,
    /// `BOT_TASK_MOVE_TO_VECTOR`: edict or vector, float = how near.
    MoveToVector,
    /// `BOT_TASK_PICKUP_ITEM`: edict = the item.
    PickupItem,
    /// `BOT_TASK_USE`: edict = what to use, int = state, float = how long.
    Use,
    /// `BOT_TASK_WAIT`: float = how long.
    Wait,
    /// `BOT_TASK_DEFEND`.
    Defend,
    /// `BOT_TASK_LISTEN_TO_SOUND`: edict = who made it, float = how long.
    ListenToSound,
    /// `BOT_TASK_CHANGE_WEAPON`: int = the weapon id.
    ChangeWeapon,
    /// `BOT_TASK_DROP_WEAPON`.
    DropWeapon,
    /// `BOT_TASK_CROUCH`: float = how long.
    Crouch,
}

impl TaskKind {
    /// `CBotTask::_getTaskDescription`.
    pub fn description(self) -> &'static str {
        match self {
            TaskKind::None => "BOT_TASK_NONE",
            TaskKind::FindPath => "BOT_TASK_FIND_PATH",
            TaskKind::NormalAttack => "BOT_TASK_NORMAL_ATTACK",
            TaskKind::SearchForEnemy => "BOT_TASK_SEARCH_FOR_ENEMY",
            TaskKind::Reload => "BOT_TASK_RELOAD",
            TaskKind::WaitAndFaceVector => "BOT_TASK_WAIT_AND_FACE_VECTOR",
            TaskKind::FaceVector => "BOT_TASK_FACE_VECTOR",
            TaskKind::MoveToVector => "BOT_TASK_MOVE_TO_VECTOR",
            TaskKind::PickupItem => "BOT_TASK_PICKUP_ITEM",
            TaskKind::Use => "BOT_TASK_USE",
            TaskKind::Wait => "BOT_TASK_WAIT",
            TaskKind::Defend => "BOT_TASK_DEFEND",
            TaskKind::ListenToSound => "BOT_TASK_LISTEN_TO_SOUND",
            TaskKind::ChangeWeapon => "BOT_TASK_CHANGE_WEAPON",
            TaskKind::DropWeapon => "BOT_TASK_DROP_WEAPON",
            TaskKind::Crouch => "BOT_TASK_CROUCH",
        }
    }
}

/// `eScheduleDesc`: the schedule descriptions the TS bot uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleDesc {
    /// `BOT_SCHED_NONE`.
    None,
    /// `BOT_SCHED_RUN_FOR_COVER`.
    RunForCover,
}

/// `CBotTask`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Task {
    /// `m_Task`.
    pub kind: TaskKind,
    /// `m_iInfo`.
    pub int: i32,
    /// `m_fInfo`.
    pub float: f32,
    /// `m_vInfo`.
    pub vector: Vector,
    /// `m_pInfo`.
    pub edict: Option<EntRef>,
    /// `m_iScheduleId` (0: none).
    pub schedule_id: i32,
    /// `m_iScheduleDescription`.
    pub schedule_desc: ScheduleDesc,
    /// `m_bFoundPath`.
    pub found_path: bool,
    /// `m_fTimeToComplete` (-1: no limit).
    pub time_to_complete: f32,
}

impl Task {
    /// `CBotTask(iTask, iScheduleId, pInfo, iInfo, fInfo, vInfo, fTimeToComplete)` with no time
    /// limit.
    pub fn new(kind: TaskKind, schedule_id: i32, edict: Option<EntRef>, int: i32, float: f32, vector: Vector) -> Task {
        Task { kind, int, float, vector, edict, schedule_id, schedule_desc: ScheduleDesc::None, found_path: false, time_to_complete: -1.0 }
    }

    /// `CBotTask(iTask)`.
    pub fn simple(kind: TaskKind) -> Task {
        Task::new(kind, 0, None, 0, 0.0, Vector::ZERO)
    }

    /// The same task with `fTimeToComplete` (seconds from `now`).
    pub fn with_timeout(mut self, now: f32, secs: f32) -> Task {
        self.time_to_complete = now + secs;
        self
    }

    /// `TimedOut`.
    pub fn timed_out(&self, now: f32) -> bool {
        self.time_to_complete != -1.0 && self.time_to_complete < now
    }

    /// `SetTimeToComplete`: seconds from `now`.
    pub fn set_time_to_complete(&mut self, now: f32, secs: f32) {
        self.time_to_complete = now + secs;
    }

    /// `CBotTask::operator==`: kind, int, float, vector and edict.
    pub fn same(&self, o: &Task) -> bool {
        self.kind == o.kind && self.int == o.int && self.float == o.float && self.vector == o.vector && self.edict == o.edict
    }
}

/// `CBotTasks`: the queue, current task at the front.
#[derive(Clone, Debug, Default)]
pub struct Tasks {
    tasks: VecDeque<Task>,
}

impl Tasks {
    /// The tasks, current first.
    pub fn iter(&self) -> impl Iterator<Item = &Task> {
        self.tasks.iter()
    }

    /// `CurrentTask`.
    pub fn current(&self) -> Option<&Task> {
        self.tasks.front()
    }

    /// `CurrentTask` for changing it.
    pub fn current_mut(&mut self) -> Option<&mut Task> {
        self.tasks.front_mut()
    }

    /// `NoTasksLeft`.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// `FlushTasks`.
    pub fn flush(&mut self) {
        self.tasks.clear();
    }

    /// `HasTask(eBotTask)`.
    pub fn has_task(&self, kind: TaskKind) -> bool {
        self.tasks.iter().any(|t| t.kind == kind)
    }

    /// `HasSchedule`.
    pub fn has_schedule(&self, desc: ScheduleDesc) -> bool {
        self.tasks.iter().any(|t| t.schedule_desc == desc)
    }

    /// `GetNewScheduleId`: the lowest id from 1 no task uses.
    pub fn new_schedule_id(&self) -> i32 {
        let mut id = 1;
        while self.tasks.iter().any(|t| t.schedule_id == id) {
            id += 1;
        }
        id
    }

    /// `FinishSchedule(int)`: removes every task of the schedule; true if any went.
    pub fn finish_schedule(&mut self, schedule_id: i32) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(|t| t.schedule_id != schedule_id);
        before != self.tasks.len()
    }

    /// `FinishSchedule(eScheduleDesc)`.
    pub fn finish_schedule_desc(&mut self, desc: ScheduleDesc) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(|t| t.schedule_desc != desc);
        before != self.tasks.len()
    }

    /// `GiveSchedIdDescription`.
    pub fn give_schedule_description(&mut self, schedule_id: i32, desc: ScheduleDesc) {
        for t in self.tasks.iter_mut().filter(|t| t.schedule_id == schedule_id) {
            t.schedule_desc = desc;
        }
    }

    /// `SetTimeToCompleteSchedule`.
    pub fn set_time_to_complete_schedule(&mut self, schedule_id: i32, now: f32, secs: f32) {
        // Upstream adds the time twice (`fTime += gpGlobals->time`, then `SetTimeToComplete`
        // adds it again), so schedules time out at `2 * now + secs`. Kept (design §5).
        let t = now + secs;
        for task in self.tasks.iter_mut().filter(|x| x.schedule_id == schedule_id) {
            task.set_time_to_complete(now, t);
        }
    }

    /// `RemoveTimedOutSchedules`: fails the schedules of timed-out tasks other than the current
    /// task's.
    pub fn remove_timed_out_schedules(&mut self, now: f32) {
        let Some(ignore) = self.current().map(|t| t.schedule_id) else { return };
        while let Some(t) = self.tasks.iter().find(|t| t.timed_out(now) && t.schedule_id != ignore).copied() {
            if !self.finish_schedule(t.schedule_id) {
                break;
            }
        }
    }

    /// `RemoveSameTask`: removes equal tasks, and their schedules.
    fn remove_same_task(&mut self, task: &Task) {
        while let Some(p) = self.tasks.iter().position(|t| t.same(task)) {
            let removed = self.tasks.remove(p).expect("position is in range");
            if removed.schedule_id != 0 {
                self.finish_schedule(removed.schedule_id);
            }
        }
    }

    /// `RemoveSimilarTask`: removes every task of a kind, and their schedules.
    pub fn remove_similar_task(&mut self, kind: TaskKind) {
        while let Some(p) = self.tasks.iter().position(|t| t.kind == kind) {
            let removed = self.tasks.remove(p).expect("position is in range");
            if removed.schedule_id != 0 {
                self.finish_schedule(removed.schedule_id);
            }
        }
    }

    /// `AddTaskToEnd` (`CBot::AddTask`).
    pub fn add_to_end(&mut self, task: Task) {
        self.tasks.push_back(task);
    }

    /// `AddTaskToFront`.
    pub fn add_to_front(&mut self, task: Task) {
        self.remove_same_task(&task);
        self.tasks.push_front(task);
    }

    /// `CBot::AddPriorityTask`: `RemovePaths` then `AddTaskToFront`.
    pub fn add_priority(&mut self, task: Task) {
        self.remove_paths();
        self.add_to_front(task);
    }

    /// `RemovePaths`: every task after the current one forgets its path.
    pub fn remove_paths(&mut self) {
        for t in self.tasks.iter_mut().skip(1) {
            t.found_path = false;
        }
    }

    /// `FinishedCurrentTask(bFailed)`: drops the current task (and on failure its schedule).
    /// After a `FIND_PATH` the next task inherits its path info.
    pub fn finished_current(&mut self, failed: bool) {
        let Some(t) = self.tasks.pop_front() else { return };
        if failed && t.schedule_id != 0 {
            self.finish_schedule(t.schedule_id);
        }
        if t.kind == TaskKind::FindPath && !self.tasks.is_empty() {
            self.remove_paths();
            if let Some(next) = self.tasks.front_mut() {
                next.found_path = t.found_path;
            }
        }
    }

    /// Clears the edict of tasks pointing at an entity that is gone (`UpdateConditions`'
    /// `FNullEnt` check does it for the current task).
    pub fn forget_edict(&mut self, e: EntRef) {
        for t in self.tasks.iter_mut().filter(|t| t.edict == Some(e)) {
            t.edict = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(kind: TaskKind, sched: i32, int: i32) -> Task {
        Task::new(kind, sched, None, int, 0.0, Vector::ZERO)
    }

    #[test]
    fn queue_order_and_schedules() {
        let mut q = Tasks::default();
        q.add_to_end(t(TaskKind::FindPath, 1, 5));
        q.add_to_end(t(TaskKind::PickupItem, 1, 0));
        q.add_to_end(t(TaskKind::Use, 2, 0));
        assert_eq!(q.current().unwrap().kind, TaskKind::FindPath);
        assert_eq!(q.new_schedule_id(), 3);
        // A failed task fails its schedule.
        q.finished_current(true);
        assert_eq!(q.current().unwrap().kind, TaskKind::Use);
        // Front insertion removes an equal task first.
        q.add_to_end(t(TaskKind::Wait, 0, 0));
        q.add_to_front(t(TaskKind::Wait, 0, 0));
        assert_eq!(q.iter().filter(|x| x.kind == TaskKind::Wait).count(), 1);
        assert_eq!(q.current().unwrap().kind, TaskKind::Wait);
    }

    #[test]
    fn find_path_hands_its_path_to_the_next_task() {
        let mut q = Tasks::default();
        let mut fp = t(TaskKind::FindPath, 1, 5);
        fp.found_path = true;
        q.add_to_end(fp);
        q.add_to_end(t(TaskKind::PickupItem, 1, 0));
        q.finished_current(false);
        assert!(q.current().unwrap().found_path);
    }

    #[test]
    fn timeouts() {
        let mut q = Tasks::default();
        q.add_to_end(t(TaskKind::Wait, 1, 0));
        q.add_to_end(t(TaskKind::FindPath, 2, 0).with_timeout(0.0, 1.0));
        q.add_to_end(t(TaskKind::Use, 2, 0));
        q.remove_timed_out_schedules(5.0);
        assert_eq!(q.iter().count(), 1);
        assert!(!q.current().unwrap().timed_out(5.0));
    }
}
