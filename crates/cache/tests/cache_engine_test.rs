use moon_cache::*;
use moon_common::path::WorkspaceRelativePathBuf;
use moon_config::CacheConfig;
use moon_env_var::GlobalEnvBag;
use starbase_sandbox::create_empty_sandbox;

mod cache_engine {
    use super::*;

    #[test]
    fn creates_cache_dir_tag() {
        let sandbox = create_empty_sandbox();

        CacheEngine::new(
            sandbox.path().join(".moon"),
            sandbox.path(),
            &CacheConfig::default(),
        )
        .unwrap();

        assert!(sandbox.path().join(".moon/cache/CACHEDIR.TAG").exists());
    }

    #[test]
    fn returns_default_if_cache_missing() {
        let sandbox = create_empty_sandbox();
        let engine = CacheEngine::new(
            sandbox.path().join(".moon"),
            sandbox.path(),
            &CacheConfig::default(),
        )
        .unwrap();
        let item = engine
            .state
            .load_state::<CommonCacheState>("state.json")
            .unwrap();

        assert_eq!(item.data, CommonCacheState::default());
    }

    #[test]
    fn reads_cache_if_exists() {
        let sandbox = create_empty_sandbox();
        sandbox.create_file(
            ".moon/cache/states/state.json",
            r#"{ "lastHash": "abc123" }"#,
        );

        let engine = CacheEngine::new(
            sandbox.path().join(".moon"),
            sandbox.path(),
            &CacheConfig::default(),
        )
        .unwrap();
        let item = engine
            .state
            .load_state::<CommonCacheState>("state.json")
            .unwrap();

        assert_eq!(
            item.data,
            CommonCacheState {
                last_hash: "abc123".into()
            }
        );
    }

    #[test]
    fn can_write_cache_if_mode_off() {
        let sandbox = create_empty_sandbox();
        let engine = CacheEngine::new(
            sandbox.path().join(".moon"),
            sandbox.path(),
            &CacheConfig::default(),
        )
        .unwrap();
        let bag = GlobalEnvBag::instance();

        bag.set("MOON_CACHE", "off");

        engine
            .write(
                "test.json",
                &CommonCacheState {
                    last_hash: "abc123".into(),
                },
            )
            .unwrap();

        assert!(sandbox.path().join(".moon/cache/test.json").exists());

        bag.remove("MOON_CACHE");
    }

    #[test]
    fn can_write_cache_if_mode_readonly() {
        let sandbox = create_empty_sandbox();
        let engine = CacheEngine::new(
            sandbox.path().join(".moon"),
            sandbox.path(),
            &CacheConfig::default(),
        )
        .unwrap();
        let bag = GlobalEnvBag::instance();

        bag.set("MOON_CACHE", "read");

        engine
            .write(
                engine.state.resolve_path("test.json"),
                &CommonCacheState {
                    last_hash: "abc123".into(),
                },
            )
            .unwrap();

        assert!(sandbox.path().join(".moon/cache/states/test.json").exists());

        bag.remove("MOON_CACHE");
    }

    mod cas_root {
        use super::*;
        use starbase_sandbox::create_empty_sandbox;
        use std::sync::Mutex;

        // Serialise tests that mutate the global X_MOON_SHARED_CAS_CACHE env bag entry.
        static LOCK: Mutex<()> = Mutex::new(());

        #[test]
        fn defaults_to_cache_dir() {
            let _guard = LOCK.lock().unwrap();
            let bag = GlobalEnvBag::instance();
            bag.remove("X_MOON_SHARED_CAS_CACHE");

            let sandbox = create_empty_sandbox();
            let engine = CacheEngine::new(
                sandbox.path().join(".moon"),
                sandbox.path(),
                &CacheConfig::default(),
            )
            .unwrap();

            assert_eq!(engine.cas_root, engine.cache_dir);
            assert!(engine.ac.objects_dir.starts_with(&engine.cache_dir));
            assert!(engine.cas.objects_dir.starts_with(&engine.cache_dir));
        }

        #[test]
        fn uses_moon_home_cache_when_shared_flag_set() {
            let _guard = LOCK.lock().unwrap();
            let sandbox = create_empty_sandbox();
            let moon_home = create_empty_sandbox();
            let bag = GlobalEnvBag::instance();

            bag.set("X_MOON_SHARED_CAS_CACHE", "true");

            let engine = CacheEngine::new(
                sandbox.path().join(".moon"),
                moon_home.path(),
                &CacheConfig::default(),
            )
            .unwrap();

            let expected_root = moon_home.path().join("cache");
            assert_eq!(engine.cas_root, expected_root);
            assert!(engine.ac.objects_dir.starts_with(&expected_root));
            assert!(engine.cas.objects_dir.starts_with(&expected_root));
            assert!(engine.cache_dir.starts_with(sandbox.path()));

            bag.remove("X_MOON_SHARED_CAS_CACHE");
        }
    }

    mod hash_files {
        use super::*;

        fn rel(path: &str) -> WorkspaceRelativePathBuf {
            WorkspaceRelativePathBuf::from(path)
        }

        #[tokio::test]
        async fn hashes_files_into_hex() {
            let sandbox = create_empty_sandbox();
            sandbox.create_file("a.txt", "hello");
            sandbox.create_file("b.txt", "world");

            let engine = CacheEngine::new(
                sandbox.path().join(".moon"),
                sandbox.path(),
                &CacheConfig::default(),
            )
            .unwrap();

            let files = vec![rel("a.txt"), rel("b.txt")];
            let result = engine.hash_files(sandbox.path(), &files).await.unwrap();

            assert_eq!(result.len(), 2);
            for path in &files {
                let hash = result.get(path).expect("missing hash");
                assert_eq!(hash.len(), 64);
                assert!(
                    hash.chars()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
                );
            }
        }

        #[tokio::test]
        async fn same_content_produces_same_hash() {
            let sandbox = create_empty_sandbox();
            sandbox.create_file("a.txt", "same");
            sandbox.create_file("b.txt", "same");

            let engine = CacheEngine::new(
                sandbox.path().join(".moon"),
                sandbox.path(),
                &CacheConfig::default(),
            )
            .unwrap();

            let result = engine
                .hash_files(sandbox.path(), &[rel("a.txt"), rel("b.txt")])
                .await
                .unwrap();

            assert_eq!(result[&rel("a.txt")], result[&rel("b.txt")]);
        }

        #[tokio::test]
        async fn different_content_produces_different_hash() {
            let sandbox = create_empty_sandbox();
            sandbox.create_file("a.txt", "one");
            sandbox.create_file("b.txt", "two");

            let engine = CacheEngine::new(
                sandbox.path().join(".moon"),
                sandbox.path(),
                &CacheConfig::default(),
            )
            .unwrap();

            let result = engine
                .hash_files(sandbox.path(), &[rel("a.txt"), rel("b.txt")])
                .await
                .unwrap();

            assert_ne!(result[&rel("a.txt")], result[&rel("b.txt")]);
        }

        #[tokio::test]
        async fn returns_empty_map_for_empty_input() {
            let sandbox = create_empty_sandbox();
            let engine = CacheEngine::new(
                sandbox.path().join(".moon"),
                sandbox.path(),
                &CacheConfig::default(),
            )
            .unwrap();

            let result = engine.hash_files(sandbox.path(), &[]).await.unwrap();

            assert!(result.is_empty());
        }

        #[tokio::test]
        async fn skips_missing_files() {
            let sandbox = create_empty_sandbox();
            sandbox.create_file("exists.txt", "here");

            let engine = CacheEngine::new(
                sandbox.path().join(".moon"),
                sandbox.path(),
                &CacheConfig::default(),
            )
            .unwrap();

            let result = engine
                .hash_files(
                    sandbox.path(),
                    &[
                        rel("exists.txt"),
                        rel("missing.txt"),
                        rel("nested/gone.txt"),
                    ],
                )
                .await
                .unwrap();

            assert_eq!(result.len(), 1);
            assert!(result.contains_key(&rel("exists.txt")));
        }

        #[tokio::test]
        async fn skips_directories() {
            let sandbox = create_empty_sandbox();
            sandbox.create_file("file.txt", "content");
            std::fs::create_dir_all(sandbox.path().join("subdir")).unwrap();

            let engine = CacheEngine::new(
                sandbox.path().join(".moon"),
                sandbox.path(),
                &CacheConfig::default(),
            )
            .unwrap();

            let result = engine
                .hash_files(sandbox.path(), &[rel("file.txt"), rel("subdir")])
                .await
                .unwrap();

            assert_eq!(result.len(), 1);
            assert!(result.contains_key(&rel("file.txt")));
            assert!(!result.contains_key(&rel("subdir")));
        }

        #[tokio::test]
        async fn hashes_files_in_nested_directories() {
            let sandbox = create_empty_sandbox();
            sandbox.create_file("top.txt", "a");
            sandbox.create_file("nested/mid.txt", "b");
            sandbox.create_file("nested/deeper/bottom.txt", "c");

            let engine = CacheEngine::new(
                sandbox.path().join(".moon"),
                sandbox.path(),
                &CacheConfig::default(),
            )
            .unwrap();

            let files = vec![
                rel("top.txt"),
                rel("nested/mid.txt"),
                rel("nested/deeper/bottom.txt"),
            ];
            let result = engine.hash_files(sandbox.path(), &files).await.unwrap();

            assert_eq!(result.len(), 3);
            for path in &files {
                assert!(result.contains_key(path));
            }
        }
    }
}
