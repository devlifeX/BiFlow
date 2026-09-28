use super::{HELPER_PIPE_SDDL, RUNTIME_PIPE_SDDL};
use std::{ffi::c_void, io, mem::size_of, ptr};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows::{
    core::HSTRING,
    Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        },
    },
};

struct OwnedSecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for OwnedSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            // SAFETY: ConvertStringSecurityDescriptorToSecurityDescriptorW
            // allocates this descriptor with LocalAlloc.
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0 .0)));
            }
        }
    }
}

/// Creates a local helper named pipe with the packaged SDDL.
///
/// # Errors
///
/// Returns an I/O error when the security descriptor cannot be built or the
/// pipe cannot be created.
pub fn create_helper_server(pipe_name: &str, first_instance: bool) -> io::Result<NamedPipeServer> {
    create_server(pipe_name, first_instance, HELPER_PIPE_SDDL)
}

/// Creates a local runtime named pipe accessible only to its owner and SYSTEM.
///
/// # Errors
///
/// Returns an I/O error when the security descriptor cannot be built or the
/// pipe cannot be created.
pub fn create_runtime_server(pipe_name: &str, first_instance: bool) -> io::Result<NamedPipeServer> {
    create_server(pipe_name, first_instance, RUNTIME_PIPE_SDDL)
}

fn create_server(
    pipe_name: &str,
    first_instance: bool,
    sddl_value: &str,
) -> io::Result<NamedPipeServer> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let sddl = HSTRING::from(sddl_value);
    // SAFETY: `sddl` stays alive for the call; `descriptor` receives a
    // LocalAlloc pointer that `OwnedSecurityDescriptor` frees.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &sddl,
            SDDL_REVISION_1,
            &raw mut descriptor,
            None,
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
    }
    let owned = OwnedSecurityDescriptor(descriptor);
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
        lpSecurityDescriptor: owned.0 .0,
        bInheritHandle: false.into(),
    };
    let mut options = ServerOptions::new();
    options.reject_remote_clients(true);
    options.first_pipe_instance(first_instance);
    // SAFETY: `attributes` points at a valid SECURITY_ATTRIBUTES for the
    // duration of CreateNamedPipeW.
    unsafe {
        options.create_with_security_attributes_raw(
            pipe_name,
            ptr::from_mut(&mut attributes).cast::<c_void>(),
        )
    }
}
