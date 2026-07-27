use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Read, Write},
    mem::{size_of, zeroed},
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    ptr, thread,
    time::{Duration, Instant},
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Threading::{CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
    },
};

use crate::EnvironmentPolicy;

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const EXIT_CLEANUP_GRACE: Duration = Duration::from_secs(2);

pub struct ProcessRequest<'a> {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
    pub working_directory: PathBuf,
    pub stdin: &'a [u8],
    pub environment: EnvironmentPolicy,
    pub timeout: Duration,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TreeCleanup {
    Clean,
    Terminated,
}

#[derive(Debug)]
pub struct ProcessResult {
    pub exit_code: Option<i32>,
    pub duration: Duration,
    pub timed_out: bool,
    pub tree_cleanup: TreeCleanup,
}

pub fn run_process(request: &ProcessRequest<'_>) -> io::Result<ProcessResult> {
    prepare_capture_path(&request.stdout_path)?;
    prepare_capture_path(&request.stderr_path)?;

    let mut command = Command::new(&request.program);
    command
        .args(&request.arguments)
        .current_dir(&request.working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_SUSPENDED);
    request.environment.apply(&mut command);

    let started = Instant::now();
    let mut child = command.spawn()?;
    let job = match Job::new().and_then(|job| job.assign(&child).map(|()| job)) {
        Ok(job) => job,
        Err(error) => {
            terminate_unassigned_child(&mut child);
            return Err(error);
        }
    };

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("child stdout pipe was not created"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("child stderr pipe was not created"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("child stdin pipe was not created"))?;

    let stdout_path = request.stdout_path.clone();
    let stdout_thread = thread::spawn(move || drain(stdout, &stdout_path));
    let stderr_path = request.stderr_path.clone();
    let stderr_thread = thread::spawn(move || drain(stderr, &stderr_path));
    let input = request.stdin.to_vec();
    let stdin_thread = thread::spawn(move || write_input(stdin, &input));

    if let Err(error) = resume_initial_thread(child.id()) {
        let _ = job.terminate();
        let _ = child.wait();
        join_worker(stdin_thread, "stdin")?;
        join_worker(stdout_thread, "stdout")?;
        join_worker(stderr_thread, "stderr")?;
        return Err(error);
    }

    let deadline = started + request.timeout;
    let (status, timed_out) = wait_until(&mut child, deadline, &job)?;
    let mut tree_cleanup = TreeCleanup::Clean;
    if !job.wait_empty(Instant::now() + EXIT_CLEANUP_GRACE)? {
        job.terminate()?;
        tree_cleanup = TreeCleanup::Terminated;
        job.wait_empty(Instant::now() + EXIT_CLEANUP_GRACE)?
            .then_some(())
            .ok_or_else(|| io::Error::other("process tree remained active after termination"))?;
    }

    join_worker(stdin_thread, "stdin")?;
    join_worker(stdout_thread, "stdout")?;
    join_worker(stderr_thread, "stderr")?;

    Ok(ProcessResult {
        exit_code: status.code(),
        duration: started.elapsed(),
        timed_out,
        tree_cleanup,
    })
}

fn prepare_capture_path(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn drain(mut reader: impl Read, path: &Path) -> io::Result<()> {
    let mut file = File::create(path)?;
    io::copy(&mut reader, &mut file)?;
    file.flush()
}

fn write_input(mut stdin: impl Write, input: &[u8]) -> io::Result<()> {
    stdin.write_all(input)?;
    stdin.flush()
}

fn join_worker(worker: thread::JoinHandle<io::Result<()>>, stream: &'static str) -> io::Result<()> {
    worker
        .join()
        .map_err(|_| io::Error::other(format!("{stream} worker panicked")))?
        .map_err(|error| io::Error::new(error.kind(), format!("{stream} worker failed: {error}")))
}

fn wait_until(child: &mut Child, deadline: Instant, job: &Job) -> io::Result<(ExitStatus, bool)> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status, false));
        }
        if Instant::now() >= deadline {
            job.terminate()?;
            return child.wait().map(|status| (status, true));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn terminate_unassigned_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn resume_initial_thread(process_id: u32) -> io::Result<()> {
    let snapshot = OwnedHandle::snapshot()?;
    let mut entry: THREADENTRY32 = unsafe { zeroed() };
    entry.dwSize = u32::try_from(size_of::<THREADENTRY32>()).expect("THREADENTRY32 size fits u32");
    let mut present = unsafe { Thread32First(snapshot.raw(), &raw mut entry) } != 0;
    while present {
        if entry.th32OwnerProcessID == process_id {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            let thread = OwnedHandle::from_nullable(thread, "could not open child initial thread")?;
            let result = unsafe { ResumeThread(thread.raw()) };
            if result == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            return Ok(());
        }
        present = unsafe { Thread32Next(snapshot.raw(), &raw mut entry) } != 0;
    }
    Err(io::Error::other("could not find child initial thread"))
}

struct Job {
    handle: OwnedHandle,
}

impl Job {
    fn new() -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        let handle = OwnedHandle::from_nullable(handle, "could not create Job Object")?;
        let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let result = unsafe {
            SetInformationJobObject(
                handle.raw(),
                JobObjectExtendedLimitInformation,
                (&raw const information).cast(),
                u32::try_from(size_of_val(&information)).expect("job information size fits u32"),
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { handle })
    }

    fn assign(&self, child: &Child) -> io::Result<()> {
        let process = child.as_raw_handle().cast();
        if unsafe { AssignProcessToJobObject(self.handle.raw(), process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn terminate(&self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.handle.raw(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn wait_empty(&self, deadline: Instant) -> io::Result<bool> {
        loop {
            if self.active_processes()? == 0 {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn active_processes(&self) -> io::Result<u32> {
        let mut information: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        let result = unsafe {
            QueryInformationJobObject(
                self.handle.raw(),
                JobObjectBasicAccountingInformation,
                (&raw mut information).cast(),
                u32::try_from(size_of_val(&information)).expect("job information size fits u32"),
                ptr::null_mut(),
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(information.ActiveProcesses)
    }
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn snapshot() -> io::Result<Self> {
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    fn from_nullable(handle: HANDLE, message: &str) -> io::Result<Self> {
        if handle.is_null() {
            let error = io::Error::last_os_error();
            return Err(io::Error::new(error.kind(), format!("{message}: {error}")));
        }
        Ok(Self(handle))
    }

    const fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::IsolatedWorkspace;

    fn request<'a>(
        root: &Path,
        arguments: Vec<OsString>,
        input: &'a [u8],
        timeout: Duration,
    ) -> ProcessRequest<'a> {
        ProcessRequest {
            program: PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
            arguments,
            working_directory: root.to_owned(),
            stdin: input,
            environment: EnvironmentPolicy::inherit(),
            timeout,
            stdout_path: root.join("stdout.txt"),
            stderr_path: root.join("stderr.txt"),
        }
    }

    #[test]
    fn captures_stdin_stdout_and_stderr() {
        let mut workspace = IsolatedWorkspace::create(Path::new("target/test-support"), "capture")
            .expect("workspace should be created");
        let script = "$value = [Console]::In.ReadToEnd(); [Console]::Out.Write($value); [Console]::Error.Write('problem')";
        let result = run_process(&request(
            workspace.root(),
            vec!["-NoProfile".into(), "-Command".into(), script.into()],
            b"hello",
            Duration::from_secs(10),
        ))
        .expect("process should run");
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.timed_out);
        assert_eq!(
            fs::read(workspace.root().join("stdout.txt")).unwrap(),
            b"hello"
        );
        assert_eq!(
            fs::read(workspace.root().join("stderr.txt")).unwrap(),
            b"problem"
        );
        workspace.finish().expect("workspace should clean up");
    }

    #[test]
    fn timeout_terminates_descendants() {
        let mut workspace = IsolatedWorkspace::create(Path::new("target/test-support"), "tree")
            .expect("workspace should be created");
        let marker = workspace.root().join("descendant-finished.txt");
        let escaped = marker.display().to_string().replace('\'', "''");
        let script = format!(
            "Start-Process powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep 5; Set-Content -NoNewline -Path ''{escaped}'' -Value alive'; Start-Sleep 30"
        );
        let result = run_process(&request(
            workspace.root(),
            vec!["-NoProfile".into(), "-Command".into(), script.into()],
            &[],
            Duration::from_millis(500),
        ))
        .expect("timed process should be controlled");
        assert!(result.timed_out);
        thread::sleep(Duration::from_secs(6));
        assert!(
            !marker.exists(),
            "descendant survived the Job Object timeout"
        );
        workspace.finish().expect("workspace should clean up");
    }
}
