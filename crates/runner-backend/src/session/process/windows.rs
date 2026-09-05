use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::time::Instant;

use portable_pty::MasterPty;
use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    CREATE_NO_WINDOW, PROCESS_ACCESS_RIGHTS, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

pub(crate) struct ProcessTree {
    job: OwnedHandle,
    process: OwnedHandle,
}

impl ProcessTree {
    pub(crate) fn adopt(pid: u32) -> io::Result<Self> {
        let job = create_job()?;
        set_job_limits(&job)?;
        let process = open_process(
            pid,
            PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
        )?;
        assign_process(&job, &process)?;
        Ok(Self { job, process })
    }

    pub(crate) fn attach_pty(&mut self, _master: &dyn MasterPty) {}

    #[allow(dead_code)]
    pub(crate) fn snapshot_descendants(&self) -> Vec<u32> {
        Vec::new()
    }

    pub(crate) fn terminate(&self) -> io::Result<()> {
        check_bool(unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) })
    }

    pub(crate) fn root_has_exited(&self) -> io::Result<bool> {
        process_has_exited(&self.process)
    }

    // Job membership includes background helpers, unlike a Unix foreground process group.
    pub(crate) fn has_other_processes(&self) -> io::Result<bool> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        check_bool(unsafe {
            QueryInformationJobObject(
                self.job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        })?;
        Ok(info.ActiveProcesses > 1)
    }
}

fn check_bool(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn create_job() -> io::Result<OwnedHandle> {
    let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn set_job_limits(job: &OwnedHandle) -> io::Result<()> {
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    check_bool(unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    })
}

fn open_process(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> io::Result<OwnedHandle> {
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn assign_process(job: &OwnedHandle, process: &OwnedHandle) -> io::Result<()> {
    check_bool(unsafe { AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) })
}

pub(crate) fn kill_process(pid: i32) -> io::Result<()> {
    let process = open_process(pid as u32, PROCESS_TERMINATE)?;
    check_bool(unsafe { TerminateProcess(process.as_raw_handle(), 1) })
}

#[allow(dead_code)]
fn process_is_active(pid: i32) -> io::Result<bool> {
    let process = open_process(pid as u32, PROCESS_SYNCHRONIZE)?;
    Ok(!process_has_exited(&process)?)
}

fn process_has_exited(process: &OwnedHandle) -> io::Result<bool> {
    match unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } {
        WAIT_TIMEOUT => Ok(false),
        WAIT_OBJECT_0 => Ok(true),
        _ => Err(io::Error::last_os_error()),
    }
}

#[allow(dead_code)]
pub(crate) fn process_exists(pid: i32) -> bool {
    process_is_active(pid).unwrap_or(false)
}

#[allow(dead_code)]
pub(crate) fn reap_descendants(_session_id: &str, _descendants: &[i32]) {}

// Windows exposes only the image path here; startup cleanup never uses it for identity.
#[allow(dead_code)]
pub(crate) fn process_command_line(pid: i32) -> io::Result<Option<String>> {
    let process = match open_process(pid as u32, PROCESS_QUERY_LIMITED_INFORMATION) {
        Ok(process) => process,
        Err(error) if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let mut buffer = vec![0u16; 32768];
    let mut len = buffer.len() as u32;
    check_bool(unsafe {
        QueryFullProcessImageNameW(process.as_raw_handle(), 0, buffer.as_mut_ptr(), &mut len)
    })?;
    Ok(Some(String::from_utf16_lossy(&buffer[..len as usize])))
}

#[allow(dead_code)]
fn wait_for_exit(pid: i32, deadline: Instant) -> io::Result<bool> {
    let process = match open_process(pid as u32, PROCESS_SYNCHRONIZE) {
        Ok(process) => process,
        Err(error) if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) => {
            return Ok(true);
        }
        Err(error) => return Err(error),
    };
    let millis = deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .min((u32::MAX - 1) as u128) as u32;
    match unsafe { WaitForSingleObject(process.as_raw_handle(), millis) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}

#[allow(dead_code)]
pub(crate) fn wait_for_process_exit_until(pid: i32, deadline: Instant) -> bool {
    wait_for_exit(pid, deadline).unwrap_or(false)
}

pub(crate) fn prepare_headless_fork(command: &mut std::process::Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

pub(crate) fn kill_headless_fork(child: &mut std::process::Child, process_tree: &ProcessTree) {
    let _ = process_tree.terminate();
    let _ = child.wait();
}

#[cfg(test)]
pub(crate) fn read_raw_console_input(len: usize) -> io::Result<Vec<u8>> {
    use std::io::Read;
    use windows_sys::Win32::System::Console::{
        ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
    };
    let mut input = std::io::stdin();
    let mode = console_input_mode(&input)?;
    set_console_input_mode(
        &input,
        (mode & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT))
            | ENABLE_VIRTUAL_TERMINAL_INPUT,
    )?;
    let mut bytes = vec![0; len];
    let read = input.read_exact(&mut bytes);
    let restore = set_console_input_mode(&input, mode);
    read?;
    restore?;
    Ok(bytes)
}

#[cfg(test)]
fn console_input_mode(input: &std::io::Stdin) -> io::Result<u32> {
    let mut mode = 0;
    check_bool(unsafe {
        windows_sys::Win32::System::Console::GetConsoleMode(input.as_raw_handle(), &mut mode)
    })?;
    Ok(mode)
}

#[cfg(test)]
fn set_console_input_mode(input: &std::io::Stdin, mode: u32) -> io::Result<()> {
    check_bool(unsafe {
        windows_sys::Win32::System::Console::SetConsoleMode(input.as_raw_handle(), mode)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    #[test]
    fn terminate_and_drop_kill_the_job_tree() {
        for terminate in [true, false] {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile", "-NonInteractive", "-Command",
                "$null=[Console]::ReadLine(); $child=Start-Process -FilePath $env:ComSpec -ArgumentList '/d /c ping -n 30 127.0.0.1 >nul' -NoNewWindow -PassThru -ErrorAction Stop; [Console]::WriteLine($child.Id); $child.WaitForExit()",
            ]).stdin(Stdio::piped()).stdout(Stdio::piped());
            prepare_headless_fork(&mut command);
            let mut child = command.spawn().unwrap();
            let tree = ProcessTree::adopt(child.id()).unwrap();
            let pid = child.id() as i32;
            assert!(process_exists(pid));
            assert!(!tree.has_other_processes().unwrap());
            assert!(!wait_for_process_exit_until(pid, Instant::now()));
            assert!(process_command_line(pid)
                .unwrap()
                .unwrap()
                .to_ascii_lowercase()
                .ends_with("powershell.exe"));
            assert!(tree.snapshot_descendants().is_empty());
            child.stdin.as_mut().unwrap().write_all(b"\r\n").unwrap();
            let mut line = String::new();
            std::io::BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut line)
                .unwrap();
            let descendant = line.trim().parse::<i32>().unwrap();
            assert!(tree.has_other_processes().unwrap());
            assert!(process_exists(descendant));
            let tree = if terminate {
                tree.terminate().unwrap();
                Some(tree)
            } else {
                drop(tree);
                None
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            assert!(wait_for_process_exit_until(pid, deadline));
            assert!(wait_for_process_exit_until(descendant, deadline));
            assert!(!process_exists(pid));
            assert!(!process_exists(descendant));
            child.wait().unwrap();
            drop(tree);
        }
    }

    #[test]
    fn kill_process_and_headless_fork_stop_children() {
        for headless in [true, false] {
            let mut command = Command::new("cmd");
            command
                .args(["/d", "/q"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null());
            prepare_headless_fork(&mut command);
            let mut child = command.spawn().unwrap();
            let pid = child.id() as i32;
            assert!(process_exists(pid));
            if headless {
                let tree = ProcessTree::adopt(child.id()).unwrap();
                kill_headless_fork(&mut child, &tree);
            } else {
                kill_process(pid).unwrap();
            }
            assert!(wait_for_process_exit_until(
                pid,
                Instant::now() + Duration::from_secs(5)
            ));
            child.wait().unwrap();
            assert!(!process_exists(pid));
        }
    }

    #[test]
    fn exit_code_259_is_not_an_active_process() {
        let mut child = Command::new("cmd")
            .args(["/c", "exit 259"])
            .spawn()
            .unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(259));
        assert!(!process_exists(child.id() as i32));
    }

    #[test]
    fn job_rejects_breakaway_children() {
        if std::env::var_os("RUNNER_TEST_BREAKAWAY_PROBE").is_some() {
            return;
        }
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session::process::windows::tests::breakaway_probe",
                "--nocapture",
            ])
            .env("RUNNER_TEST_BREAKAWAY_PROBE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let tree = ProcessTree::adopt(child.id()).unwrap();
        child.stdin.take().unwrap().write_all(b"x").unwrap();
        let exited = wait_for_process_exit_until(
            child.id() as i32,
            Instant::now() + Duration::from_secs(10),
        );
        drop(tree);
        let output = child.wait_with_output().unwrap();
        assert!(exited, "breakaway probe did not finish");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("runner-breakaway-denied"));
    }

    #[test]
    fn breakaway_probe() {
        if std::env::var_os("RUNNER_TEST_BREAKAWAY_PROBE").is_none() {
            return;
        }
        use std::io::Read;
        use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
        use windows_sys::Win32::System::Threading::CREATE_BREAKAWAY_FROM_JOB;
        std::io::stdin().read_exact(&mut [0]).unwrap();
        let error = Command::new("cmd")
            .args(["/c", "exit 0"])
            .creation_flags(CREATE_BREAKAWAY_FROM_JOB)
            .spawn()
            .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
        println!("runner-breakaway-denied");
    }
}
