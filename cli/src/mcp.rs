use std::path::PathBuf;

use tokio::net::UnixStream;

const APP_IDENTIFIER: &str = "com.wycstudios.runner";

fn socket_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    #[cfg(target_os = "macos")]
    let base = home.join("Library/Application Support");
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    Some(base.join(APP_IDENTIFIER).join("mcp.sock"))
}

pub fn run() -> i32 {
    let path = match socket_path() {
        Some(p) => p,
        None => {
            eprintln!("runner mcp: cannot resolve app data directory");
            return 1;
        }
    };

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runner mcp: failed to start runtime: {e}");
            return 1;
        }
    };

    rt.block_on(async {
        let stream = match UnixStream::connect(&path).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "runner mcp: cannot connect to Runner.app at {}: {e}\n\
                     Is Runner running?",
                    path.display()
                );
                return 1;
            }
        };

        let (mut sock_read, mut sock_write) = stream.into_split();
        let mut stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();

        let to_sock = tokio::io::copy(&mut stdin, &mut sock_write);
        let from_sock = tokio::io::copy(&mut sock_read, &mut stdout);

        tokio::select! {
            r = to_sock => {
                if let Err(e) = r {
                    eprintln!("runner mcp: stdin→socket: {e}");
                    return 1;
                }
            }
            r = from_sock => {
                if let Err(e) = r {
                    eprintln!("runner mcp: socket→stdout: {e}");
                    return 1;
                }
            }
        }
        0
    })
}
