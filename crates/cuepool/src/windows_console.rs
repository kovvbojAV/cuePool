//! Let an explicit CLI request use its parent's console without creating one.

use std::ffi::c_void;

const STD_OUTPUT_HANDLE: u32 = (-11_i32) as u32;
const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
const INVALID_HANDLE_VALUE: *mut c_void = (-1_isize) as *mut c_void;
const FILE_TYPE_UNKNOWN: u32 = 0;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetStdHandle(std_handle: u32) -> *mut c_void;
    fn GetFileType(file: *mut c_void) -> u32;
    fn SetLastError(error: u32);
    fn GetLastError() -> u32;
    fn AttachConsole(process_id: u32) -> i32;
}

/// Call before printing CLI output, never on the normal GUI startup path.
pub(crate) fn attach_parent_for_cli() {
    // SAFETY: These Win32 calls inspect the process-owned standard handle or
    // attach this process to an existing console. We neither close nor own the
    // handle. NULL and INVALID_HANDLE_VALUE are checked before inspecting it.
    unsafe {
        let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
        if !stdout.is_null() && stdout != INVALID_HANDLE_VALUE {
            // Preserve inherited pipes, files, NUL and usable console handles.
            // Merely checking is_terminal() would also reject valid redirection.
            // GetFileType can return UNKNOWN successfully; only an error means
            // the inherited handle is unusable.
            // https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-getfiletype
            SetLastError(0);
            if GetFileType(stdout) != FILE_TYPE_UNKNOWN || GetLastError() == 0 {
                return;
            }
        }

        // GUI executables may start without standard handles. Attachment can
        // supply them when invoked from a terminal. Failure is normal when the
        // parent has no console; never allocate a new window for this request.
        // https://learn.microsoft.com/windows/console/attachconsole
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
