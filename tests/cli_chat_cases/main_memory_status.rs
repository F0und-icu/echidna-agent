use super::*;

#[test]
fn memory_status_reads_current_or_legacy_project_without_model_or_automatic_writes() {
    for legacy in [false, true] {
        let root = TemporaryDirectory::new("main-memory-status");
        initialize_project(root.path());
        if legacy {
            fs::rename(
                root.path().join(".echidna"),
                root.path().join(".cyber-agent"),
            )
            .unwrap();
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let config = write_config(
            root.path(),
            &format!("http://{}/v1", listener.local_addr().unwrap()),
        );
        let memory = root
            .path()
            .canonicalize()
            .unwrap()
            .join(if legacy { ".cyber-agent" } else { ".echidna" })
            .join("memory");
        let output = run_chat(root.path(), &config, "/memory\n/exit\n", &[]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&format!("\"directory\":\"{}\"", memory.display())),
            "{stdout}"
        );
        assert!(stdout.contains("\"core_present\":false"), "{stdout}");
        assert!(
            stdout.contains("\"automatic_extraction\":false"),
            "{stdout}"
        );
        assert!(!memory.exists(), "status must not create Memory");
        fs::create_dir(&memory).unwrap();
        fs::write(memory.join("core.md"), "Explicit stable project note.").unwrap();
        let output = run_chat(root.path(), &config, "/memory\n/exit\n", &[]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("\"core_present\":true"), "{stdout}");
        assert!(stdout.contains("\"core_bytes\":29"), "{stdout}");
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}
