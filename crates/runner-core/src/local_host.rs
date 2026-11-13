/// Whether a `file://` authority names this machine: empty, `localhost`,
/// this machine's hostname, or its first label, ignoring ASCII case.
pub fn is_local_host(host: &str) -> bool {
    is_local_host_for(host, local_hostname().as_deref())
}

pub fn is_local_host_for(host: &str, local_name: Option<&str>) -> bool {
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    local_name.is_some_and(|name| {
        !name.is_empty()
            && (host.eq_ignore_ascii_case(name)
                || name
                    .split('.')
                    .next()
                    .is_some_and(|label| !label.is_empty() && host.eq_ignore_ascii_case(label)))
    })
}

#[cfg(unix)]
pub fn local_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer outlives the call and its length is passed with it.
    let status = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if status != 0 {
        return None;
    }
    let end = buf.iter().position(|&byte| byte == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec())
        .ok()
        .filter(|name| !name.is_empty())
}

#[cfg(windows)]
pub fn local_hostname() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.is_empty())
}
