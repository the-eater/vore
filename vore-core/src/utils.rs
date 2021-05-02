use anyhow::Context;
use std::ffi::{CStr, CString};

pub fn get_username_by_uid(uid: u32) -> anyhow::Result<Option<String>> {
    unsafe {
        let passwd = libc::getpwuid(uid);
        if !passwd.is_null() {
            return Ok(Some(
                CStr::from_ptr((*passwd).pw_name)
                    .to_str()
                    .with_context(|| {
                        format!("Username of user with uid {} is not valid UTF-8", uid)
                    })
                    .map(|x| x.to_string())?,
            ));
        } else {
            Ok(None)
        }
    }
}

pub fn get_uid_by_username(username: &str) -> anyhow::Result<u32> {
    unsafe {
        let c_str = CString::new(username)?;
        let passwd = libc::getpwnam(c_str.as_ptr());
        if passwd.is_null() {
            anyhow::bail!("No user found with the name {}", username);
        }

        Ok((*passwd).pw_uid)
    }
}
