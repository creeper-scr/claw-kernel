//! Memory bridge — exposes the memory store to Lua scripts.

use std::sync::Arc;

use claw_memory::{MemoryItem, MemoryStore};
use claw_provider::embedding::{Embedder, NgramEmbedder};
use mlua::{Lua, Result as LuaResult, UserData, UserDataMethods};

/// Memory bridge exposing the memory store to Lua scripts.
///
/// Memory is automatically namespaced by `agent_id`, so scripts cannot
/// accidentally access another agent's memory.
///
/// Registered as the global `memory` table.
///
/// # Example in Lua:
/// ```lua
/// -- Store a value
/// memory:set("user_preference", "concise")
///
/// -- Retrieve by key
/// local val = memory:get("user_preference")
/// if val then
///     print("Preference:", val)
/// end
///
/// -- Semantic search
/// local results = memory:search("user preferences", 5)
/// for _, item in ipairs(results) do
///     print(item.content)
/// end
/// ```
pub struct MemoryBridge {
    store: Arc<dyn MemoryStore>,
    embedder: Arc<NgramEmbedder>,
    /// Namespace = agent_id; not visible to scripts.
    namespace: String,
}

impl MemoryBridge {
    /// Create a new MemoryBridge with the given store and namespace.
    pub fn new(store: Arc<dyn MemoryStore>, namespace: impl Into<String>) -> Self {
        Self {
            store,
            embedder: Arc::new(NgramEmbedder::new()),
            namespace: namespace.into(),
        }
    }

    /// Build the deterministic memory ID for a key in this namespace.
    fn item_id(&self, key: &str) -> claw_memory::MemoryId {
        claw_memory::MemoryId::new(format!("{}::{}", self.namespace, key))
    }
}

impl UserData for MemoryBridge {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        // set(key, value) — stores a string value under the given key.
        methods.add_async_method(
            "set",
            |_lua, this, (key, value): (String, String)| async move {
                let id = this.item_id(&key);
                // Delete existing entry if present (upsert semantics).
                let _ = this.store.delete(&id).await;

                let embedding = this.embedder.embed(&value);
                let mut item = MemoryItem::new(this.namespace.clone(), value);
                item.id = id;
                item.embedding = Some(embedding);

                this.store
                    .store(item)
                    .await
                    .map_err(|e| mlua::Error::RuntimeError(format!("memory set error: {}", e)))?;

                Ok(())
            },
        );

        // get(key) -> string | nil
        methods.add_async_method("get", |_lua, this, key: String| async move {
            let id = this.item_id(&key);
            match this.store.retrieve(&id).await {
                Ok(Some(item)) => Ok(Some(item.content)),
                Ok(None) => Ok(None),
                Err(e) => Err(mlua::Error::RuntimeError(format!(
                    "memory get error: {}",
                    e
                ))),
            }
        });

        // delete(key) -> nil
        methods.add_async_method("delete", |_lua, this, key: String| async move {
            let id = this.item_id(&key);
            this.store
                .delete(&id)
                .await
                .map_err(|e| mlua::Error::RuntimeError(format!("memory delete error: {}", e)))?;
            Ok(())
        });

        // search(query, top_k) -> [{id, content, importance}]
        methods.add_async_method(
            "search",
            |lua, this, (query, top_k): (String, usize)| async move {
                let embedding = this.embedder.embed(&query);
                let results = this
                    .store
                    .semantic_search(&embedding, top_k)
                    .await
                    .map_err(|e| {
                        mlua::Error::RuntimeError(format!("memory search error: {}", e))
                    })?;

                // Filter to this namespace only.
                let results: Vec<_> = results
                    .into_iter()
                    .filter(|item| item.namespace == this.namespace)
                    .collect();

                let table = lua.create_table()?;
                for (i, item) in results.into_iter().enumerate() {
                    let entry = lua.create_table()?;
                    entry.set("id", item.id.as_str().to_string())?;
                    entry.set("content", item.content)?;
                    entry.set("importance", item.importance as f64)?;
                    table.raw_set(i + 1, entry)?;
                }
                Ok(table)
            },
        );
    }
}

/// Register the MemoryBridge as a global `memory` table in the Lua instance.
pub fn register_memory(lua: &Lua, bridge: MemoryBridge) -> LuaResult<()> {
    lua.globals().set("memory", bridge)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use claw_memory::{error::MemoryError, traits::MemoryStore, types::*};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MockStore(Mutex<HashMap<String, MemoryItem>>);

    impl MockStore {
        fn new() -> Arc<Self> {
            Arc::new(Self(Mutex::new(HashMap::new())))
        }
    }

    #[async_trait]
    impl MemoryStore for MockStore {
        async fn store(&self, item: MemoryItem) -> Result<MemoryId, MemoryError> {
            let id = item.id.clone();
            self.0.lock().unwrap().insert(id.as_str().to_string(), item);
            Ok(id)
        }

        async fn retrieve(&self, id: &MemoryId) -> Result<Option<MemoryItem>, MemoryError> {
            Ok(self.0.lock().unwrap().get(id.as_str()).cloned())
        }

        async fn search_episodic(
            &self,
            _filter: &EpisodicFilter,
        ) -> Result<Vec<EpisodicEntry>, MemoryError> {
            Ok(vec![])
        }

        async fn semantic_search(
            &self,
            _query: &[f32],
            top_k: usize,
        ) -> Result<Vec<MemoryItem>, MemoryError> {
            let store = self.0.lock().unwrap();
            Ok(store.values().take(top_k).cloned().collect())
        }

        async fn delete(&self, id: &MemoryId) -> Result<(), MemoryError> {
            self.0.lock().unwrap().remove(id.as_str());
            Ok(())
        }

        async fn clear_namespace(&self, ns: &str) -> Result<usize, MemoryError> {
            let mut store = self.0.lock().unwrap();
            let before = store.len();
            store.retain(|_, v| v.namespace != ns);
            Ok(before - store.len())
        }

        async fn namespace_usage(&self, _ns: &str) -> Result<u64, MemoryError> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn test_memory_bridge_set_and_get() {
        let store = MockStore::new();
        let bridge = MemoryBridge::new(store, "agent-1");

        let lua = unsafe { Lua::unsafe_new() };
        register_memory(&lua, bridge).unwrap();

        // set a value
        lua.load(
            r#"
            local ok = memory:set("pref", "concise")
        "#,
        )
        .exec_async()
        .await
        .unwrap();

        // get the value back
        let result: String = lua
            .load(r#"return memory:get("pref")"#)
            .eval_async()
            .await
            .unwrap();
        assert_eq!(result, "concise");
    }

    #[tokio::test]
    async fn test_memory_bridge_get_missing_returns_nil() {
        let store = MockStore::new();
        let bridge = MemoryBridge::new(store, "agent-1");

        let lua = unsafe { Lua::unsafe_new() };
        register_memory(&lua, bridge).unwrap();

        let result: bool = lua
            .load(r#"return memory:get("nonexistent") == nil"#)
            .eval_async()
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_memory_bridge_set_overwrites() {
        let store = MockStore::new();
        let bridge = MemoryBridge::new(store, "agent-1");

        let lua = unsafe { Lua::unsafe_new() };
        register_memory(&lua, bridge).unwrap();

        lua.load(r#"memory:set("key", "first")"#)
            .exec_async()
            .await
            .unwrap();
        lua.load(r#"memory:set("key", "second")"#)
            .exec_async()
            .await
            .unwrap();

        let result: String = lua
            .load(r#"return memory:get("key")"#)
            .eval_async()
            .await
            .unwrap();
        assert_eq!(result, "second");
    }

    #[tokio::test]
    async fn test_memory_bridge_delete() {
        let store = MockStore::new();
        let bridge = MemoryBridge::new(store, "agent-1");

        let lua = unsafe { Lua::unsafe_new() };
        register_memory(&lua, bridge).unwrap();

        lua.load(r#"memory:set("temp", "value")"#)
            .exec_async()
            .await
            .unwrap();
        lua.load(r#"memory:delete("temp")"#)
            .exec_async()
            .await
            .unwrap();

        let result: bool = lua
            .load(r#"return memory:get("temp") == nil"#)
            .eval_async()
            .await
            .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_memory_bridge_search_returns_table() {
        let store = MockStore::new();
        let bridge = MemoryBridge::new(store, "agent-1");

        let lua = unsafe { Lua::unsafe_new() };
        register_memory(&lua, bridge).unwrap();

        lua.load(r#"memory:set("item1", "hello world")"#)
            .exec_async()
            .await
            .unwrap();

        let result: bool = lua
            .load(
                r#"
                local results = memory:search("hello", 5)
                return type(results) == "table"
            "#,
            )
            .eval_async()
            .await
            .unwrap();
        assert!(result);
    }

    #[test]
    fn test_memory_bridge_item_id_format() {
        let store = MockStore::new();
        let bridge = MemoryBridge::new(store, "agent-42");
        let id = bridge.item_id("my_key");
        assert_eq!(id.as_str(), "agent-42::my_key");
    }
}
