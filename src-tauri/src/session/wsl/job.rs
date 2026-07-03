//! App-wide Windows Job Object so spawned `wsl.exe` relays — and the
//! in-distro agent trees behind them — die with the app process no
//! matter how it exits.
//!
//! Graceful quit already kills tracked sessions (lib.rs
//! `stop_running_sessions_on_quit`). This covers the path that hook
//! can't: a crash, a `taskkill /F`, or any abnormal exit. Without it a
//! killed `runner.exe` orphans its `wsl.exe` children; the in-WSL
//! `claude`/`codex` keep running, accumulate, and break
//! `claude --continue` resumes by competing for the same session.
//!
//! Mechanism: a single job is created lazily and held in a process-
//! lifetime `OnceLock`. The OS closes that handle when the process
//! exits, and `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` then terminates every
//! assigned process. Each spawned relay is assigned right after spawn.
//! Best-effort throughout: a failure logs and is ignored rather than
//! blocking a spawn.

use std::ptr;
use std::sync::OnceLock;

use winapi::shared::minwindef::FALSE;
use winapi::um::handleapi::CloseHandle;
use winapi::um::jobapi2::{
    AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
};
use winapi::um::processthreadsapi::OpenProcess;
use winapi::um::winnt::{
    HANDLE, JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
};

/// Wrapper so the raw `HANDLE` (a `*mut c_void`) can live in a `static`.
/// We never close it: it must stay open for the whole process so the OS
/// teardown is what closes it (triggering KILL_ON_JOB_CLOSE).
struct JobHandle(HANDLE);
unsafe impl Send for JobHandle {}
unsafe impl Sync for JobHandle {}

static APP_JOB: OnceLock<Option<JobHandle>> = OnceLock::new();

fn app_job() -> HANDLE {
    let slot = APP_JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(ptr::null_mut(), ptr::null());
        if job.is_null() {
            log::warn!("wsl/job: CreateJobObject failed; agents won't be reaped on crash");
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == FALSE {
            log::warn!("wsl/job: SetInformationJobObject failed; closing job");
            CloseHandle(job);
            return None;
        }
        log::info!("wsl/job: app job created (KILL_ON_JOB_CLOSE)");
        Some(JobHandle(job))
    });
    match slot {
        Some(JobHandle(h)) => *h,
        None => ptr::null_mut(),
    }
}

/// Assign the spawned process (by PID) to the app job so it is killed
/// when `runner.exe` exits by any means. Best-effort.
pub fn assign_to_app_job(pid: u32) {
    let job = app_job();
    if job.is_null() {
        return;
    }
    unsafe {
        let proc = OpenProcess(PROCESS_TERMINATE | PROCESS_SET_QUOTA, FALSE, pid);
        if proc.is_null() {
            log::warn!("wsl/job: OpenProcess({pid}) failed; relay not reaped on crash");
            return;
        }
        if AssignProcessToJobObject(job, proc) == FALSE {
            log::warn!("wsl/job: AssignProcessToJobObject({pid}) failed");
        }
        CloseHandle(proc);
    }
}
