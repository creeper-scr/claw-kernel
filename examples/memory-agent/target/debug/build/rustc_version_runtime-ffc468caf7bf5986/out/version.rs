
            /// Returns the `rustc` SemVer version and additional metadata
            /// like the git short hash and build date.
            pub fn version_meta() -> VersionMeta {
                VersionMeta {
                    semver: Version {
                        major: 1,
                        minor: 83,
                        patch: 0,
                        pre: Prerelease::new("").unwrap(),
                        build: BuildMetadata::new("").unwrap(),
                    },
                    host: "aarch64-apple-darwin".to_owned(),
                    short_version_string: "rustc 1.83.0 (90b35a623 2024-11-26)".to_owned(),
                    commit_hash: Some("90b35a6239c3d8bdabc530a6a0816f7ff89a0aaf".to_owned()),
                    commit_date: Some("2024-11-26".to_owned()),
                    build_date: None,
                    channel: Channel::Stable,
                    llvm_version: Some(LlvmVersion{ major: 19, minor: 1 }),
                }
            }
            