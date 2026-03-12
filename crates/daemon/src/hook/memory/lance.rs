//! LanceDB graph storage for the memory hook.
//!
//! Three tables: `entities` (typed nodes with FTS), `relations` (directed
//! edges between entities), and `journals` (compaction summaries with vector
//! embeddings for semantic search). Mutations use lancedb directly; graph
//! traversal uses lance-graph Cypher queries via `DirNamespace`. Entities and
//! relations are shared across all agents (DD#40). Journals are agent-scoped.

use anyhow::Result;
use arrow_array::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray,
    UInt64Array, cast::AsArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lance_graph::{CypherQuery, DirNamespace, GraphConfig};
use lancedb::{
    Connection, Table as LanceTable, connect,
    index::{Index, scalar::FullTextSearchQuery},
    query::{ExecutableQuery, QueryBase},
};
use std::{collections::HashMap, path::Path, sync::Arc};

const ENTITIES_TABLE: &str = "entities";
const RELATIONS_TABLE: &str = "relations";
const JOURNALS_TABLE: &str = "journals";
const CONNECTIONS_MAX: usize = 100;

/// Embedding vector dimension (all-MiniLM-L6-v2).
pub(crate) const EMBED_DIM: i32 = 384;

/// Row data for an entity.
pub(crate) struct EntityRow<'a> {
    pub id: &'a str,
    pub entity_type: &'a str,
    pub key: &'a str,
    pub value: &'a str,
    pub vector: Vec<f32>,
}

/// Row data for a relation.
pub(crate) struct RelationRow<'a> {
    pub source: &'a str,
    pub relation: &'a str,
    pub target: &'a str,
}

/// An entity returned from queries.
pub(crate) struct EntityResult {
    pub id: String,
    pub entity_type: String,
    pub key: String,
    pub value: String,
    pub created_at: u64,
}

/// A relation returned from queries.
pub(crate) struct RelationResult {
    pub source: String,
    pub relation: String,
    pub target: String,
}

/// A journal entry returned from queries.
pub(crate) struct JournalResult {
    pub summary: String,
    pub created_at: u64,
}

/// LanceDB graph store with entities and relations tables.
///
/// Mutations use lancedb's merge_insert directly. Graph traversal
/// (`find_connections`) uses lance-graph Cypher queries.
pub(crate) struct LanceStore {
    _db: Connection,
    entities: LanceTable,
    relations: LanceTable,
    journals: LanceTable,
    namespace: Arc<DirNamespace>,
    graph_config: GraphConfig,
}

impl LanceStore {
    /// Open or create the LanceDB database with entities and relations tables.
    ///
    /// Detects v1 schema (entity table has `agent` column) and migrates to v2.
    /// Detects v2 schema (entities without `vector` column) and backfills
    /// embeddings using the provided embed function.
    pub async fn open<F>(path: impl AsRef<Path>, embed_fn: F) -> Result<Self>
    where
        F: Fn(&str) -> Result<Vec<f32>>,
    {
        let path = path.as_ref();
        let db = connect(path.to_str().unwrap_or(".")).execute().await?;

        let mut entities = open_or_create(&db, ENTITIES_TABLE, entity_schema()).await?;
        let mut relations = open_or_create(&db, RELATIONS_TABLE, relation_schema()).await?;
        let journals = open_or_create(&db, JOURNALS_TABLE, journal_schema()).await?;

        // Detect v1 schema and migrate if needed.
        let schema = entities.schema().await?;
        let has_agent = schema.fields().iter().any(|f| f.name() == "agent");
        if has_agent {
            tracing::info!("detected v1 schema — migrating entities and relations");
            let (e, r) = migrate_v1_to_v2(&db, &entities, &relations, &embed_fn).await?;
            entities = e;
            relations = r;
            tracing::info!("v1 → v2 migration complete");
        } else {
            // Detect v2 schema (no vector column) and backfill.
            let has_vector = schema.fields().iter().any(|f| f.name() == "vector");
            if !has_vector {
                tracing::info!("detected v2 schema — backfilling entity embeddings");
                entities = backfill_entity_vectors(&db, &entities, &embed_fn).await?;
                tracing::info!("entity vector backfill complete");
            }
        }

        let namespace = Arc::new(DirNamespace::new(path.to_str().unwrap_or(".")));
        let graph_config = GraphConfig::builder()
            .with_node_label(ENTITIES_TABLE, "id")
            .with_relationship(RELATIONS_TABLE, "source", "target")
            .build()?;

        let store = Self {
            _db: db,
            entities,
            relations,
            journals,
            namespace,
            graph_config,
        };
        store.ensure_entity_indices().await;
        store.ensure_relation_indices().await;
        store.ensure_journal_indices().await;
        Ok(store)
    }

    /// Upsert an entity by its id.
    ///
    /// Note: `when_matched_update_all` resets `created_at` on update.
    /// LanceDB merge_insert does not support column exclusion, and a
    /// read-before-write adds a round-trip per upsert. `updated_at`
    /// tracks the last modification time; `created_at` is best-effort.
    pub async fn upsert_entity(&self, row: &EntityRow<'_>) -> Result<()> {
        let batch = make_entity_batch(row)?;
        let schema = batch.schema();
        let batches = RecordBatchIterator::new(std::iter::once(Ok(batch)), schema);

        let mut merge = self.entities.merge_insert(&["id"]);
        merge
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        merge.execute(Box::new(batches)).await?;
        Ok(())
    }

    /// Full-text search on entities with optional type filter.
    pub async fn search_entities(
        &self,
        query: &str,
        entity_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EntityResult>> {
        let mut q = self
            .entities
            .query()
            .full_text_search(FullTextSearchQuery::new(query.to_owned()));
        if let Some(et) = entity_type {
            q = q.only_if(format!("entity_type = '{}'", escape_sql(et)));
        }
        let batches: Vec<RecordBatch> = q.limit(limit).execute().await?.try_collect().await?;
        batches_to_entities(&batches)
    }

    /// Semantic search on entities by vector similarity.
    pub async fn search_entities_semantic(
        &self,
        query_vector: &[f32],
        entity_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EntityResult>> {
        let mut q = self.entities.query().nearest_to(query_vector)?;
        if let Some(et) = entity_type {
            q = q.only_if(format!("entity_type = '{}'", escape_sql(et)));
        }
        let batches: Vec<RecordBatch> = q.limit(limit).execute().await?.try_collect().await?;
        batches_to_entities(&batches)
    }

    /// Query entities by type (no FTS, returns all matching).
    pub async fn query_by_type(
        &self,
        entity_type: &str,
        limit: usize,
    ) -> Result<Vec<EntityResult>> {
        let filter = format!("entity_type = '{}'", escape_sql(entity_type));
        let batches: Vec<RecordBatch> = self
            .entities
            .query()
            .only_if(filter)
            .limit(limit)
            .execute()
            .await?
            .try_collect()
            .await?;
        batches_to_entities(&batches)
    }

    /// Look up an entity by key.
    pub async fn find_entity_by_key(&self, key: &str) -> Result<Option<EntityResult>> {
        let filter = format!("key = '{}'", escape_sql(key));
        let batches: Vec<RecordBatch> = self
            .entities
            .query()
            .only_if(filter)
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;
        let entities = batches_to_entities(&batches)?;
        Ok(entities.into_iter().next())
    }

    /// List entities with optional type filter (no FTS).
    pub async fn list_entities(
        &self,
        entity_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EntityResult>> {
        let mut q = self.entities.query();
        if let Some(et) = entity_type {
            q = q.only_if(format!("entity_type = '{}'", escape_sql(et)));
        }
        let batches: Vec<RecordBatch> = q.limit(limit).execute().await?.try_collect().await?;
        batches_to_entities(&batches)
    }

    /// Upsert a relation (deduplicated by source+relation+target).
    pub async fn upsert_relation(&self, row: &RelationRow<'_>) -> Result<()> {
        let batch = make_relation_batch(row)?;
        let schema = batch.schema();
        let batches = RecordBatchIterator::new(std::iter::once(Ok(batch)), schema);

        let mut merge = self
            .relations
            .merge_insert(&["source", "relation", "target"]);
        merge
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        merge.execute(Box::new(batches)).await?;
        Ok(())
    }

    /// Find 1-hop connections from/to an entity using lance-graph Cypher.
    pub async fn find_connections(
        &self,
        entity_id: &str,
        relation: Option<&str>,
        direction: Direction,
        limit: usize,
    ) -> Result<Vec<RelationResult>> {
        let limit = limit.min(CONNECTIONS_MAX);
        let cypher = build_connections_cypher(entity_id, relation, direction, limit);
        let query = CypherQuery::new(&cypher)?.with_config(self.graph_config.clone());
        let batch = query
            .execute_with_namespace_arc(Arc::clone(&self.namespace), None)
            .await?;

        batch_to_relations(&batch)
    }

    /// List relations with optional entity filter (matches source or target).
    pub async fn list_relations(
        &self,
        entity_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RelationResult>> {
        let mut q = self.relations.query();
        if let Some(eid) = entity_id {
            let escaped = escape_sql(eid);
            q = q.only_if(format!("source = '{escaped}' OR target = '{escaped}'"));
        }
        let batches: Vec<RecordBatch> = q.limit(limit).execute().await?.try_collect().await?;
        batches_to_relation_results(&batches)
    }

    /// Create indices on the entities table. Errors are non-fatal.
    async fn ensure_entity_indices(&self) {
        let idx = [
            (
                vec!["key", "value"],
                Index::FTS(Default::default()),
                "entities FTS",
            ),
            (vec!["id"], Index::BTree(Default::default()), "entities id"),
            (
                vec!["key"],
                Index::BTree(Default::default()),
                "entities key",
            ),
            (
                vec!["entity_type"],
                Index::Bitmap(Default::default()),
                "entities entity_type",
            ),
        ];
        for (cols, index, name) in idx {
            if let Err(e) = self.entities.create_index(&cols, index).execute().await {
                tracing::warn!("{name} index creation skipped: {e}");
            }
        }
    }

    /// Insert a journal entry with its embedding vector.
    pub async fn insert_journal(&self, agent: &str, summary: &str, vector: Vec<f32>) -> Result<()> {
        let batch = make_journal_batch(agent, summary, vector)?;
        let schema = batch.schema();
        let batches = RecordBatchIterator::new(std::iter::once(Ok(batch)), schema);
        self.journals.add(Box::new(batches)).execute().await?;
        Ok(())
    }

    /// Semantic search on journal entries by vector similarity.
    pub async fn search_journals(
        &self,
        query_vector: &[f32],
        agent: &str,
        limit: usize,
    ) -> Result<Vec<JournalResult>> {
        let filter = format!("agent = '{}'", escape_sql(agent));
        let batches: Vec<RecordBatch> = self
            .journals
            .query()
            .nearest_to(query_vector)?
            .only_if(filter)
            .limit(limit)
            .execute()
            .await?
            .try_collect()
            .await?;
        batches_to_journals(&batches)
    }

    /// Query most recent journal entries for an agent.
    pub async fn recent_journals(&self, agent: &str, limit: usize) -> Result<Vec<JournalResult>> {
        let filter = format!("agent = '{}'", escape_sql(agent));
        let batches: Vec<RecordBatch> = self
            .journals
            .query()
            .only_if(filter)
            .limit(limit)
            .execute()
            .await?
            .try_collect()
            .await?;
        let mut results = batches_to_journals(&batches)?;
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(results)
    }

    /// Create indices on the journals table. Errors are non-fatal.
    async fn ensure_journal_indices(&self) {
        let idx = [
            (
                vec!["agent"],
                Index::Bitmap(Default::default()),
                "journals agent",
            ),
            (vec!["id"], Index::BTree(Default::default()), "journals id"),
        ];
        for (cols, index, name) in idx {
            if let Err(e) = self.journals.create_index(&cols, index).execute().await {
                tracing::warn!("{name} index creation skipped: {e}");
            }
        }
    }

    /// Create indices on the relations table. Errors are non-fatal.
    async fn ensure_relation_indices(&self) {
        let idx = [
            (
                vec!["source"],
                Index::BTree(Default::default()),
                "relations source",
            ),
            (
                vec!["target"],
                Index::BTree(Default::default()),
                "relations target",
            ),
            (
                vec!["relation"],
                Index::Bitmap(Default::default()),
                "relations relation",
            ),
        ];
        for (cols, index, name) in idx {
            if let Err(e) = self.relations.create_index(&cols, index).execute().await {
                tracing::warn!("{name} index creation skipped: {e}");
            }
        }
    }
}

/// Direction for connection queries.
pub(crate) enum Direction {
    Outgoing,
    Incoming,
    Both,
}

// ── Helpers ─────────────────────────────────────────────────────────────

async fn open_or_create(db: &Connection, name: &str, schema: Arc<Schema>) -> Result<LanceTable> {
    match db.open_table(name).execute().await {
        Ok(t) => Ok(t),
        Err(_) => {
            let batches = RecordBatchIterator::new(std::iter::empty(), Arc::clone(&schema));
            Ok(db.create_table(name, Box::new(batches)).execute().await?)
        }
    }
}

/// Backfill entity embeddings for tables that lack a vector column.
///
/// Reads all entities, embeds `"{key} {value}"`, drops and recreates the
/// table with the new schema including vectors.
async fn backfill_entity_vectors<F>(
    db: &Connection,
    entities: &LanceTable,
    embed_fn: &F,
) -> Result<LanceTable>
where
    F: Fn(&str) -> Result<Vec<f32>>,
{
    let batches: Vec<RecordBatch> = entities.query().execute().await?.try_collect().await?;
    // (id, entity_type, key, value, vector, created_at, updated_at)
    #[allow(clippy::type_complexity)]
    let mut rows: Vec<(String, String, String, String, Vec<f32>, u64, u64)> = Vec::new();
    for batch in &batches {
        let ids = migration_col(batch, "id")?.as_string::<i32>();
        let types = migration_col(batch, "entity_type")?.as_string::<i32>();
        let keys = migration_col(batch, "key")?.as_string::<i32>();
        let values = migration_col(batch, "value")?.as_string::<i32>();
        let created =
            migration_col(batch, "created_at")?.as_primitive::<arrow_array::types::UInt64Type>();
        let updated =
            migration_col(batch, "updated_at")?.as_primitive::<arrow_array::types::UInt64Type>();
        for i in 0..batch.num_rows() {
            let key = keys.value(i);
            let value = values.value(i);
            let text = format!("{key} {value}");
            let vector = embed_fn(&text)?;
            rows.push((
                ids.value(i).to_string(),
                types.value(i).to_string(),
                key.to_string(),
                value.to_string(),
                vector,
                created.value(i),
                updated.value(i),
            ));
        }
    }

    let count = rows.len();
    tracing::info!("backfilling {count} entities with embeddings");

    db.drop_table(ENTITIES_TABLE, &[]).await?;
    let schema = entity_schema();
    if rows.is_empty() {
        let batches = RecordBatchIterator::new(std::iter::empty(), Arc::clone(&schema));
        return Ok(db
            .create_table(ENTITIES_TABLE, Box::new(batches))
            .execute()
            .await?);
    }

    let mut ids = Vec::with_capacity(count);
    let mut types = Vec::with_capacity(count);
    let mut keys_vec = Vec::with_capacity(count);
    let mut values = Vec::with_capacity(count);
    let mut all_vectors: Vec<f32> = Vec::with_capacity(count * EMBED_DIM as usize);
    let mut created_ats = Vec::with_capacity(count);
    let mut updated_ats = Vec::with_capacity(count);
    for (id, et, key, value, vector, crt, upd) in rows {
        ids.push(id);
        types.push(et);
        keys_vec.push(key);
        values.push(value);
        all_vectors.extend(vector);
        created_ats.push(crt);
        updated_ats.push(upd);
    }

    let float_array = Float32Array::from(all_vectors);
    let field = Arc::new(Field::new("item", DataType::Float32, true));
    let vector_array = FixedSizeListArray::new(field, EMBED_DIM, Arc::new(float_array), None);

    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from(ids)) as Arc<dyn Array>,
            Arc::new(StringArray::from(types)) as Arc<dyn Array>,
            Arc::new(StringArray::from(keys_vec)) as Arc<dyn Array>,
            Arc::new(StringArray::from(values)) as Arc<dyn Array>,
            Arc::new(vector_array) as Arc<dyn Array>,
            Arc::new(UInt64Array::from(created_ats)) as Arc<dyn Array>,
            Arc::new(UInt64Array::from(updated_ats)) as Arc<dyn Array>,
        ],
    )?;
    let batches = RecordBatchIterator::new(std::iter::once(Ok(batch)), schema);
    Ok(db
        .create_table(ENTITIES_TABLE, Box::new(batches))
        .execute()
        .await?)
}

/// Migrate v1 (agent-scoped) entities and relations to v2 (shared graph).
///
/// Reads all rows, rewrites entity IDs from `{agent}:{type}:{key}` to
/// `{type}:{key}` using a deterministic remap table built from entity_type
/// and key columns (avoids ambiguity with colon-containing keys).
/// Deduplicates, drops old tables, creates new ones. Embeds entities.
async fn migrate_v1_to_v2<F>(
    db: &Connection,
    entities: &LanceTable,
    relations: &LanceTable,
    embed_fn: &F,
) -> Result<(LanceTable, LanceTable)>
where
    F: Fn(&str) -> Result<Vec<f32>>,
{
    // ── Migrate entities ──────────────────────────────────────────────
    let entity_batches: Vec<RecordBatch> = entities.query().execute().await?.try_collect().await?;

    // Build old_id → new_id remap table, and deduplicate by (type, key).
    let mut id_remap: HashMap<String, String> = HashMap::new();
    let mut deduped: HashMap<(String, String), (String, u64, u64)> = HashMap::new();
    for batch in &entity_batches {
        let ids = migration_col(batch, "id")?.as_string::<i32>();
        let types = migration_col(batch, "entity_type")?.as_string::<i32>();
        let keys = migration_col(batch, "key")?.as_string::<i32>();
        let values = migration_col(batch, "value")?.as_string::<i32>();
        let updated =
            migration_col(batch, "updated_at")?.as_primitive::<arrow_array::types::UInt64Type>();
        let created =
            migration_col(batch, "created_at")?.as_primitive::<arrow_array::types::UInt64Type>();

        for i in 0..batch.num_rows() {
            let old_id = ids.value(i).to_string();
            let et = types.value(i).to_string();
            let key = keys.value(i).to_string();
            let new_id = format!("{et}:{key}");
            id_remap.insert(old_id, new_id);

            let value = values.value(i).to_string();
            let upd = updated.value(i);
            let crt = created.value(i);
            let map_key = (et, key);
            let entry = deduped.entry(map_key).or_insert((value.clone(), crt, upd));
            if upd > entry.2 {
                *entry = (value, crt, upd);
            }
        }
    }

    let entity_count = deduped.len();
    tracing::info!("migrating {entity_count} deduplicated entities");

    db.drop_table(ENTITIES_TABLE, &[]).await?;
    let schema = entity_schema();
    let new_entities = if deduped.is_empty() {
        let batches = RecordBatchIterator::new(std::iter::empty(), Arc::clone(&schema));
        db.create_table(ENTITIES_TABLE, Box::new(batches))
            .execute()
            .await?
    } else {
        let mut ids = Vec::with_capacity(entity_count);
        let mut types = Vec::with_capacity(entity_count);
        let mut keys_vec = Vec::with_capacity(entity_count);
        let mut values = Vec::with_capacity(entity_count);
        let mut all_vectors: Vec<f32> = Vec::with_capacity(entity_count * EMBED_DIM as usize);
        let mut created_ats = Vec::with_capacity(entity_count);
        let mut updated_ats = Vec::with_capacity(entity_count);

        for ((et, key), (value, crt, upd)) in &deduped {
            let text = format!("{key} {value}");
            let vector = embed_fn(&text)?;
            ids.push(format!("{et}:{key}"));
            types.push(et.clone());
            keys_vec.push(key.clone());
            values.push(value.clone());
            all_vectors.extend(vector);
            created_ats.push(*crt);
            updated_ats.push(*upd);
        }

        let float_array = Float32Array::from(all_vectors);
        let field = Arc::new(Field::new("item", DataType::Float32, true));
        let vector_array = FixedSizeListArray::new(field, EMBED_DIM, Arc::new(float_array), None);

        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(ids)) as Arc<dyn Array>,
                Arc::new(StringArray::from(types)) as Arc<dyn Array>,
                Arc::new(StringArray::from(keys_vec)) as Arc<dyn Array>,
                Arc::new(StringArray::from(values)) as Arc<dyn Array>,
                Arc::new(vector_array) as Arc<dyn Array>,
                Arc::new(UInt64Array::from(created_ats)) as Arc<dyn Array>,
                Arc::new(UInt64Array::from(updated_ats)) as Arc<dyn Array>,
            ],
        )?;
        let batches = RecordBatchIterator::new(std::iter::once(Ok(batch)), schema);
        db.create_table(ENTITIES_TABLE, Box::new(batches))
            .execute()
            .await?
    };

    // ── Migrate relations ─────────────────────────────────────────────
    let relation_batches: Vec<RecordBatch> =
        relations.query().execute().await?.try_collect().await?;

    // Deduplicate by (source, relation, target) after remapping IDs.
    let mut rel_deduped: HashMap<(String, String, String), u64> = HashMap::new();
    for batch in &relation_batches {
        let sources = migration_col(batch, "source")?.as_string::<i32>();
        let rels = migration_col(batch, "relation")?.as_string::<i32>();
        let targets = migration_col(batch, "target")?.as_string::<i32>();
        let created =
            migration_col(batch, "created_at")?.as_primitive::<arrow_array::types::UInt64Type>();

        for i in 0..batch.num_rows() {
            let raw_source = sources.value(i);
            let raw_target = targets.value(i);
            let rel = rels.value(i).to_string();
            let crt = created.value(i);

            // Use the remap table for deterministic ID rewriting.
            let source = id_remap
                .get(raw_source)
                .cloned()
                .unwrap_or_else(|| raw_source.to_string());
            let target = id_remap
                .get(raw_target)
                .cloned()
                .unwrap_or_else(|| raw_target.to_string());

            rel_deduped.entry((source, rel, target)).or_insert(crt);
        }
    }

    let rel_count = rel_deduped.len();
    tracing::info!("migrating {rel_count} deduplicated relations");

    db.drop_table(RELATIONS_TABLE, &[]).await?;
    let rel_schema = relation_schema();
    let new_relations = if rel_deduped.is_empty() {
        let batches = RecordBatchIterator::new(std::iter::empty(), Arc::clone(&rel_schema));
        db.create_table(RELATIONS_TABLE, Box::new(batches))
            .execute()
            .await?
    } else {
        let mut sources = Vec::with_capacity(rel_count);
        let mut rels = Vec::with_capacity(rel_count);
        let mut targets = Vec::with_capacity(rel_count);
        let mut created_ats = Vec::with_capacity(rel_count);

        for ((source, rel, target), crt) in &rel_deduped {
            sources.push(source.clone());
            rels.push(rel.clone());
            targets.push(target.clone());
            created_ats.push(*crt);
        }

        let batch = RecordBatch::try_new(
            Arc::clone(&rel_schema),
            vec![
                Arc::new(StringArray::from(sources)) as Arc<dyn Array>,
                Arc::new(StringArray::from(rels)) as Arc<dyn Array>,
                Arc::new(StringArray::from(targets)) as Arc<dyn Array>,
                Arc::new(UInt64Array::from(created_ats)) as Arc<dyn Array>,
            ],
        )?;
        let batches = RecordBatchIterator::new(std::iter::once(Ok(batch)), rel_schema);
        db.create_table(RELATIONS_TABLE, Box::new(batches))
            .execute()
            .await?
    };

    Ok((new_entities, new_relations))
}

fn entity_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("entity_type", DataType::Utf8, false),
        Field::new("key", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                EMBED_DIM,
            ),
            false,
        ),
        Field::new("created_at", DataType::UInt64, false),
        Field::new("updated_at", DataType::UInt64, false),
    ]))
}

fn relation_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("source", DataType::Utf8, false),
        Field::new("relation", DataType::Utf8, false),
        Field::new("target", DataType::Utf8, false),
        Field::new("created_at", DataType::UInt64, false),
    ]))
}

fn journal_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("agent", DataType::Utf8, false),
        Field::new("summary", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                EMBED_DIM,
            ),
            false,
        ),
        Field::new("created_at", DataType::UInt64, false),
    ]))
}

fn make_journal_batch(agent: &str, summary: &str, vector: Vec<f32>) -> Result<RecordBatch> {
    let schema = journal_schema();
    let now = now_unix();
    let id = format!("{agent}:{now}");
    let values = Float32Array::from(vector);
    let field = Arc::new(Field::new("item", DataType::Float32, true));
    let vector_array = FixedSizeListArray::new(field, EMBED_DIM, Arc::new(values), None);
    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![id.as_str()])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![agent])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![summary])) as Arc<dyn Array>,
            Arc::new(vector_array) as Arc<dyn Array>,
            Arc::new(UInt64Array::from(vec![now])) as Arc<dyn Array>,
        ],
    )?)
}

fn batches_to_journals(batches: &[RecordBatch]) -> Result<Vec<JournalResult>> {
    let mut results = Vec::new();
    for batch in batches {
        let summaries = batch
            .column_by_name("summary")
            .ok_or_else(|| anyhow::anyhow!("missing column: summary"))?
            .as_string::<i32>();
        let timestamps = batch
            .column_by_name("created_at")
            .ok_or_else(|| anyhow::anyhow!("missing column: created_at"))?
            .as_primitive::<arrow_array::types::UInt64Type>();
        for i in 0..batch.num_rows() {
            results.push(JournalResult {
                summary: summaries.value(i).to_string(),
                created_at: timestamps.value(i),
            });
        }
    }
    Ok(results)
}

fn make_entity_batch(row: &EntityRow<'_>) -> Result<RecordBatch> {
    let schema = entity_schema();
    let now = now_unix();
    let values = Float32Array::from(row.vector.clone());
    let field = Arc::new(Field::new("item", DataType::Float32, true));
    let vector_array = FixedSizeListArray::new(field, EMBED_DIM, Arc::new(values), None);
    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![row.id])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![row.entity_type])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![row.key])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![row.value])) as Arc<dyn Array>,
            Arc::new(vector_array) as Arc<dyn Array>,
            Arc::new(UInt64Array::from(vec![now])) as Arc<dyn Array>,
            Arc::new(UInt64Array::from(vec![now])) as Arc<dyn Array>,
        ],
    )?)
}

fn make_relation_batch(row: &RelationRow<'_>) -> Result<RecordBatch> {
    let schema = relation_schema();
    let now = now_unix();
    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![row.source])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![row.relation])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![row.target])) as Arc<dyn Array>,
            Arc::new(UInt64Array::from(vec![now])) as Arc<dyn Array>,
        ],
    )?)
}

fn batches_to_entities(batches: &[RecordBatch]) -> Result<Vec<EntityResult>> {
    let mut results = Vec::new();
    for batch in batches {
        let ids = batch
            .column_by_name("id")
            .ok_or_else(|| anyhow::anyhow!("missing column: id"))?
            .as_string::<i32>();
        let types = batch
            .column_by_name("entity_type")
            .ok_or_else(|| anyhow::anyhow!("missing column: entity_type"))?
            .as_string::<i32>();
        let keys = batch
            .column_by_name("key")
            .ok_or_else(|| anyhow::anyhow!("missing column: key"))?
            .as_string::<i32>();
        let values = batch
            .column_by_name("value")
            .ok_or_else(|| anyhow::anyhow!("missing column: value"))?
            .as_string::<i32>();
        let timestamps = batch
            .column_by_name("created_at")
            .ok_or_else(|| anyhow::anyhow!("missing column: created_at"))?
            .as_primitive::<arrow_array::types::UInt64Type>();
        for i in 0..batch.num_rows() {
            results.push(EntityResult {
                id: ids.value(i).to_string(),
                entity_type: types.value(i).to_string(),
                key: keys.value(i).to_string(),
                value: values.value(i).to_string(),
                created_at: timestamps.value(i),
            });
        }
    }
    Ok(results)
}

/// Convert relation batches from direct table queries (not Cypher).
fn batches_to_relation_results(batches: &[RecordBatch]) -> Result<Vec<RelationResult>> {
    let mut results = Vec::new();
    for batch in batches {
        let sources = batch
            .column_by_name("source")
            .ok_or_else(|| anyhow::anyhow!("missing column: source"))?
            .as_string::<i32>();
        let relations = batch
            .column_by_name("relation")
            .ok_or_else(|| anyhow::anyhow!("missing column: relation"))?
            .as_string::<i32>();
        let targets = batch
            .column_by_name("target")
            .ok_or_else(|| anyhow::anyhow!("missing column: target"))?
            .as_string::<i32>();
        for i in 0..batch.num_rows() {
            results.push(RelationResult {
                source: sources.value(i).to_string(),
                relation: relations.value(i).to_string(),
                target: targets.value(i).to_string(),
            });
        }
    }
    Ok(results)
}

fn batch_to_relations(batch: &RecordBatch) -> Result<Vec<RelationResult>> {
    if batch.num_rows() == 0 {
        return Ok(Vec::new());
    }
    // lance-graph qualifies columns as {variable}__{field} (lowercase).
    // The Cypher query binds the relationship to variable `r`.
    let sources = batch
        .column_by_name("r__source")
        .ok_or_else(|| anyhow::anyhow!("missing column: r__source"))?
        .as_string::<i32>();
    let relations = batch
        .column_by_name("r__relation")
        .ok_or_else(|| anyhow::anyhow!("missing column: r__relation"))?
        .as_string::<i32>();
    let targets = batch
        .column_by_name("r__target")
        .ok_or_else(|| anyhow::anyhow!("missing column: r__target"))?
        .as_string::<i32>();
    Ok((0..batch.num_rows())
        .map(|i| RelationResult {
            source: sources.value(i).to_string(),
            relation: relations.value(i).to_string(),
            target: targets.value(i).to_string(),
        })
        .collect())
}

/// Build a Cypher query for 1-hop connection traversal.
fn build_connections_cypher(
    entity_id: &str,
    relation: Option<&str>,
    direction: Direction,
    limit: usize,
) -> String {
    let eid = escape_cypher(entity_id);

    let rel_type = relation
        .map(|r| format!(":`{}`", escape_cypher_ident(r)))
        .unwrap_or_default();

    let pattern = match direction {
        Direction::Outgoing => {
            format!("(e:entities {{id: '{eid}'}})-[r:relations{rel_type}]->(t:entities)")
        }
        Direction::Incoming => {
            format!("(e:entities)<-[r:relations{rel_type}]-(s:entities {{id: '{eid}'}})")
        }
        Direction::Both => {
            format!("(e:entities)-[r:relations{rel_type}]-(o:entities {{id: '{eid}'}})")
        }
    };

    format!("MATCH {pattern} RETURN r.source, r.relation, r.target LIMIT {limit}")
}

/// Get a column by name from a RecordBatch, returning an error if missing.
fn migration_col<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Arc<dyn Array>> {
    batch
        .column_by_name(name)
        .ok_or_else(|| anyhow::anyhow!("migration: missing column '{name}'"))
}

fn escape_sql(s: &str) -> String {
    s.replace('\'', "''")
}

fn escape_cypher(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Escape a Cypher identifier for backtick quoting.
fn escape_cypher_ident(s: &str) -> String {
    s.replace('`', "``")
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs()
}
