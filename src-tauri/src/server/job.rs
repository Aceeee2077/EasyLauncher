//! Windows keeps all managed Java children in a kill-on-close Job Object.
//! Even an abrupt EasyLauncher termination cannot leave these Java processes orphaned.
use crate::error::{AppError, Result};
#[cfg(windows)]
pub struct ProcessGroup(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for ProcessGroup {}
#[cfg(windows)]
unsafe impl Sync for ProcessGroup {}
#[cfg(windows)]
impl ProcessGroup {
    pub fn new() -> Result<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(AppError::new(
                    "Could not create a safe Java process group.",
                    std::io::Error::last_os_error(),
                ));
            }
            let group = Self(job);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            ) == 0
            {
                return Err(AppError::new(
                    "Could not configure Java process cleanup.",
                    std::io::Error::last_os_error(),
                ));
            }
            Ok(group)
        }
    }
    pub fn attach(&self, child: &tokio::process::Child) -> Result<()> {
        let handle = child
            .raw_handle()
            .ok_or_else(|| AppError::user("Java exited before its process could be tracked."))?;
        unsafe {
            if windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(self.0, handle) == 0
            {
                return Err(AppError::new(
                    "Could not safely track Java. The process was cancelled.",
                    std::io::Error::last_os_error(),
                ));
            }
        }
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(not(windows))]
pub struct ProcessGroup;
#[cfg(not(windows))]
impl ProcessGroup {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
    pub fn attach(&self, _child: &tokio::process::Child) -> Result<()> {
        Ok(())
    }
}
