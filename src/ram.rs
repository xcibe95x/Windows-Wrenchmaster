//! Windows memory counters and a small, process-oriented working-set view.
//!
//! The system snapshot uses the documented `GetPerformanceInfo` API. Its
//! counters overlap and should not be presented as a partition of physical
//! memory. Process working sets are useful for finding large consumers, but
//! shared pages mean their sum does not equal system physical memory use.

use std::process;

/// System-wide RAMMap-style empty actions. Three use the Windows native
/// memory-list interface; Windows does not publish a supported Win32 wrapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RamMapAction {
    WorkingSets,
    SystemWorkingSet,
    ModifiedPageList,
    StandbyList,
    PriorityZeroStandbyList,
}

impl RamMapAction {
    pub fn title(self) -> &'static str {
        match self {
            Self::WorkingSets => "Empty Working Sets",
            Self::SystemWorkingSet => "Empty System Working Set",
            Self::ModifiedPageList => "Empty Modified Page List",
            Self::StandbyList => "Empty Standby List",
            Self::PriorityZeroStandbyList => "Empty Priority 0 Standby List",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Self::WorkingSets => {
                "Trims active memory from processes across Windows. Apps may stall while their pages are loaded again."
            }
            Self::SystemWorkingSet => {
                "Flushes the system file-cache working set. File reads may be slower while Windows rebuilds its cache."
            }
            Self::ModifiedPageList => {
                "Requests writeback of modified memory pages. This may cause heavy disk activity."
            }
            Self::StandbyList => {
                "Drops cached standby pages Windows could otherwise reuse quickly. Opening cached files again may be slower."
            }
            Self::PriorityZeroStandbyList => {
                "Drops only lowest-priority standby cache pages. Reopening those files may be slower."
            }
        }
    }
}

/// Run one RAMMap-compatible global empty action. The memory-list commands
/// use the Windows native NtSetSystemInformation interface, which is not a
/// stable documented Win32 API and can be denied by Windows policy.
#[cfg(windows)]
pub fn empty_rammap_action(action: RamMapAction) -> Result<(), String> {
    use std::ffi::c_void;

    if action == RamMapAction::SystemWorkingSet {
        let _privilege = PrivilegeGuard::enable("SeIncreaseQuotaPrivilege")?;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetSystemFileCacheSize(minimum: usize, maximum: usize, flags: u32) -> i32;
        }
        // Microsoft documents SIZE_T(-1) for both sizes as a cache flush;
        // flags=0 preserves the existing hard-limit configuration.
        let succeeded = unsafe { SetSystemFileCacheSize(usize::MAX, usize::MAX, 0) } != 0;
        return if succeeded {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        };
    }

    let _privilege = (action != RamMapAction::ModifiedPageList)
        .then(|| PrivilegeGuard::enable("SeProfileSingleProcessPrivilege"))
        .transpose()?;
    const SYSTEM_MEMORY_LIST_INFORMATION: i32 = 80;
    let command: u32 = match action {
        RamMapAction::WorkingSets => 2,             // MemoryEmptyWorkingSets
        RamMapAction::ModifiedPageList => 3,        // MemoryFlushModifiedList
        RamMapAction::StandbyList => 4,             // MemoryPurgeStandbyList
        RamMapAction::PriorityZeroStandbyList => 5, // MemoryPurgeLowPriorityStandbyList
        RamMapAction::SystemWorkingSet => unreachable!(),
    };

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSetSystemInformation(
            information_class: i32,
            information: *const c_void,
            information_length: u32,
        ) -> i32;
    }
    let status = unsafe {
        NtSetSystemInformation(
            SYSTEM_MEMORY_LIST_INFORMATION,
            (&command as *const u32).cast(),
            std::mem::size_of_val(&command) as u32,
        )
    };
    if status >= 0 {
        Ok(())
    } else {
        Err(format!(
            "Windows returned NTSTATUS 0x{:08X}.",
            status as u32
        ))
    }
}

#[cfg(not(windows))]
pub fn empty_rammap_action(_action: RamMapAction) -> Result<(), String> {
    Err("System-wide memory-list actions are available on Windows only.".into())
}

#[cfg(windows)]
struct PrivilegeGuard {
    token: *mut std::ffi::c_void,
    previous: TokenPrivileges,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
struct Luid {
    low_part: u32,
    high_part: i32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
struct LuidAndAttributes {
    luid: Luid,
    attributes: u32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
struct TokenPrivileges {
    privilege_count: u32,
    privileges: [LuidAndAttributes; 1],
}

#[cfg(windows)]
impl PrivilegeGuard {
    fn enable(name: &str) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        use std::ptr;

        const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
        const TOKEN_QUERY: u32 = 0x0008;
        const SE_PRIVILEGE_ENABLED: u32 = 0x0002;
        const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
            fn GetLastError() -> u32;
            fn SetLastError(error: u32);
        }
        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn OpenProcessToken(
                process: *mut std::ffi::c_void,
                desired_access: u32,
                token: *mut *mut std::ffi::c_void,
            ) -> i32;
            fn LookupPrivilegeValueW(
                system_name: *const u16,
                name: *const u16,
                luid: *mut Luid,
            ) -> i32;
            fn AdjustTokenPrivileges(
                token: *mut std::ffi::c_void,
                disable_all: i32,
                new_state: *const TokenPrivileges,
                buffer_length: u32,
                previous_state: *mut TokenPrivileges,
                return_length: *mut u32,
            ) -> i32;
        }

        let mut token = ptr::null_mut();
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let wide_name: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut luid = Luid {
            low_part: 0,
            high_part: 0,
        };
        if unsafe { LookupPrivilegeValueW(ptr::null(), wide_name.as_ptr(), &mut luid) } == 0 {
            let error = std::io::Error::last_os_error().to_string();
            unsafe { CloseHandle(token) };
            return Err(error);
        }
        let new_state = TokenPrivileges {
            privilege_count: 1,
            privileges: [LuidAndAttributes {
                luid,
                attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let mut previous = new_state;
        let mut returned = 0;
        unsafe { SetLastError(0) };
        if unsafe {
            AdjustTokenPrivileges(
                token,
                0,
                &new_state,
                std::mem::size_of::<TokenPrivileges>() as u32,
                &mut previous,
                &mut returned,
            )
        } == 0
        {
            let error = std::io::Error::last_os_error().to_string();
            unsafe { CloseHandle(token) };
            return Err(error);
        }
        if unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED {
            unsafe { CloseHandle(token) };
            return Err(format!(
                "Windows did not grant the {name} privilege. Run Wrenchmaster as administrator."
            ));
        }
        Ok(Self { token, previous })
    }
}

#[cfg(windows)]
impl Drop for PrivilegeGuard {
    fn drop(&mut self) {
        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn AdjustTokenPrivileges(
                token: *mut std::ffi::c_void,
                disable_all: i32,
                new_state: *const TokenPrivileges,
                buffer_length: u32,
                previous_state: *mut TokenPrivileges,
                return_length: *mut u32,
            ) -> i32;
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        unsafe {
            AdjustTokenPrivileges(
                self.token,
                0,
                &self.previous,
                std::mem::size_of::<TokenPrivileges>() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            CloseHandle(self.token);
        }
    }
}

/// A snapshot of documented Windows memory counters, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub physical_total_bytes: u64,
    pub physical_available_bytes: u64,
    pub system_cache_bytes: u64,
    pub kernel_paged_bytes: u64,
    pub kernel_nonpaged_bytes: u64,
    pub commit_used_bytes: u64,
    pub commit_limit_bytes: u64,
}

/// Memory usage for one process. `working_set_bytes` is resident memory and
/// can include pages shared with other processes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessMemory {
    pub pid: u32,
    pub name: String,
    pub working_set_bytes: u64,
}

/// Read system counters through the documented Windows `GetPerformanceInfo`
/// API. The snapshot reports an error on non-Windows platforms.
#[cfg(windows)]
pub fn memory_snapshot() -> Result<MemorySnapshot, String> {
    #[repr(C)]
    struct PerformanceInformation {
        cb: u32,
        commit_total: usize,
        commit_limit: usize,
        _commit_peak: usize,
        physical_total: usize,
        physical_available: usize,
        system_cache: usize,
        _kernel_total: usize,
        kernel_paged: usize,
        kernel_nonpaged: usize,
        page_size: usize,
        _handle_count: u32,
        _process_count: u32,
        _thread_count: u32,
    }

    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetPerformanceInfo(info: *mut PerformanceInformation, size: u32) -> i32;
    }

    let mut info = PerformanceInformation {
        cb: std::mem::size_of::<PerformanceInformation>() as u32,
        commit_total: 0,
        commit_limit: 0,
        _commit_peak: 0,
        physical_total: 0,
        physical_available: 0,
        system_cache: 0,
        _kernel_total: 0,
        kernel_paged: 0,
        kernel_nonpaged: 0,
        page_size: 0,
        _handle_count: 0,
        _process_count: 0,
        _thread_count: 0,
    };

    // SAFETY: `info` points to a correctly sized PERFORMANCE_INFORMATION
    // layout for the current process and remains writable for the call.
    let succeeded = unsafe {
        GetPerformanceInfo(
            &mut info,
            std::mem::size_of::<PerformanceInformation>() as u32,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }

    let bytes = |pages: usize| (pages as u64).saturating_mul(info.page_size as u64);
    Ok(MemorySnapshot {
        physical_total_bytes: bytes(info.physical_total),
        physical_available_bytes: bytes(info.physical_available),
        system_cache_bytes: bytes(info.system_cache),
        kernel_paged_bytes: bytes(info.kernel_paged),
        kernel_nonpaged_bytes: bytes(info.kernel_nonpaged),
        commit_used_bytes: bytes(info.commit_total),
        commit_limit_bytes: bytes(info.commit_limit),
    })
}

/// Unsupported-platform fallback for [`memory_snapshot`].
#[cfg(not(windows))]
pub fn memory_snapshot() -> Result<MemorySnapshot, String> {
    Err("System memory counters are available on Windows only.".into())
}

/// Return up to ten non-core processes with the largest working sets.
/// Core PIDs 0–4 and this application are excluded. Values come from the
/// supplied, already refreshed `sysinfo::System` snapshot.
pub fn top_processes(system: &sysinfo::System) -> Vec<ProcessMemory> {
    let current_pid = process::id();
    let mut processes: Vec<_> = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let pid = pid.as_u32();
            if pid <= 4 || pid == current_pid {
                return None;
            }
            Some(ProcessMemory {
                pid,
                name: process.name().to_string_lossy().into_owned(),
                working_set_bytes: process.memory(),
            })
        })
        .collect();
    processes.sort_unstable_by(|a, b| b.working_set_bytes.cmp(&a.working_set_bytes));
    processes.truncate(10);
    processes
}

/// Ask Windows to trim one process's working set.
///
/// This does not reduce committed memory; pages may need to be read back if
/// the process uses them again. PID 0–4 and this application are rejected.
#[cfg(windows)]
pub fn trim_working_set(pid: u32) -> Result<(), String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA,
    };

    if pid <= 4 || pid == process::id() {
        return Err("Refusing to trim a core system process or Wrenchmaster.".into());
    }

    #[link(name = "psapi")]
    unsafe extern "system" {
        fn EmptyWorkingSet(process: windows::Win32::Foundation::HANDLE) -> i32;
    }

    // SAFETY: The requested rights are the documented minimum required by
    // EmptyWorkingSet; `pid` was checked above.
    let handle = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION, false, pid) }
        .map_err(|error| error.to_string())?;

    // SAFETY: `handle` is an open process handle with the required rights.
    let trimmed = unsafe { EmptyWorkingSet(handle) } != 0;
    let trim_error = (!trimmed).then(std::io::Error::last_os_error);
    // Always close the acquired handle, regardless of whether trimming worked.
    let close_result = unsafe { CloseHandle(handle) };

    if let Some(error) = trim_error {
        return Err(error.to_string());
    }
    close_result.map_err(|error| error.to_string())
}

/// Unsupported-platform fallback for [`trim_working_set`].
#[cfg(not(windows))]
pub fn trim_working_set(_pid: u32) -> Result<(), String> {
    Err("Working-set trimming is available on Windows only.".into())
}
