use std::process::{Command, Stdio};

#[test]
fn enrollment_refused_before_daemon_without_terminals_or_inside_agents() {
    for action in ["login", "pin", "reset-identity"] {
        for agent in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_runner-agent-cli"));
            command
                .args(["remote", action])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env_remove("RUNNER_HANDLE");
            if agent {
                command.env("RUNNER_HANDLE", "remote-guard-test");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(1));
            assert!(String::from_utf8(output.stderr)
                .unwrap()
                .contains("interactive terminal outside an agent session"));
        }
    }
}
#[test]
fn remote_has_clap_help_but_is_absent_from_the_agent_guide() {
    let binary = env!("CARGO_BIN_EXE_runner-agent-cli");
    let help = Command::new(binary)
        .args(["remote", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains("login"));
    for args in [vec!["help"], vec!["help", "agents"]] {
        let guide = Command::new(binary).args(args).output().unwrap();
        assert!(guide.status.success());
        assert!(!String::from_utf8(guide.stdout)
            .unwrap()
            .contains("runner remote"));
    }
}

#[cfg(unix)]
#[test]
fn pin_and_reset_refuse_an_agent_even_with_a_terminal() {
    use std::os::fd::FromRawFd;
    for action in ["pin", "reset-identity"] {
        let (mut master, mut slave) = (-1, -1);
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let _master = unsafe { std::fs::File::from_raw_fd(master) };
        let slave = unsafe { std::fs::File::from_raw_fd(slave) };
        let output = Command::new(env!("CARGO_BIN_EXE_runner-agent-cli"))
            .args(["remote", action])
            .env("RUNNER_HANDLE", "remote-guard-test")
            .stdin(slave.try_clone().unwrap())
            .stdout(slave)
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8(output.stderr)
            .unwrap()
            .contains("interactive terminal outside an agent session"));
    }
}
