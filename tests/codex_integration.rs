use std::fs;
use std::path::Path;

#[test]
fn codex_skill_is_discoverable_and_matches_the_manifest() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join("dist/framewatch.json"))
            .expect("dist/framewatch.json should be packaged"),
    )
    .expect("dist/framewatch.json should be valid JSON");

    let skill_path = manifest["codex"]["skill"]
        .as_str()
        .expect("manifest should declare codex.skill");
    assert_eq!(skill_path, ".agents/skills/framewatch/SKILL.md");

    let skill = fs::read_to_string(root.join(skill_path))
        .expect("the declared repo-scoped Codex skill should exist");
    assert!(skill.starts_with("---\nname: framewatch\ndescription: "));
    assert!(!skill.contains("TODO"));
    assert!(skill.contains("framewatch shot"));
    assert!(skill.contains("framewatch watch"));
    assert!(skill.contains("Managed whisper.cpp"));

    let managed = &manifest["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["name"] == "record")
        .unwrap()["managed_transcription"];
    assert_eq!(managed["engine"], "whisper.cpp");
    assert_eq!(managed["version"], "1.9.2");
    assert_eq!(managed["model"], "base.en");
    assert_eq!(managed["cache_override_env"], "FRAMEWATCH_WHISPER_DIR");

    let metadata = fs::read_to_string(root.join(".agents/skills/framewatch/agents/openai.yaml"))
        .expect("the Codex skill UI metadata should exist");
    assert!(metadata.contains("$framewatch"));

    let agents = fs::read_to_string(root.join("AGENTS.md"))
        .expect("repository-level Codex guidance should exist");
    assert!(agents.contains(".agents/skills/framewatch/"));
}
