//! LanceDB graph storage for the memory hook.
//!
//! Three tables: `entities` (typed nodes with FTS), `relations` (directed
//! edges between entities), and `journals` (compaction summaries with vector
//! embeddings for semantic search). Mutations use lancedb directly; graph
//! traversal uses lance-graph Cypher queries via `DirNamespace`. All
//! operations scoped by agent name.

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
use std::{path::Path, sync::Arc};

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
    pub agent: &'a str,
}

/// Row data for a relation.
pub(crate) struct RelationRow<'a> {
    pub source: &'a str,
    pub relation: &'a str,
    pub target: &'a str,
    pub agent: &'a str,
}

/// An entity returned from queries.
pub(crate) struct EntityResult {
    pub id: String,
    pub entity_type: String,
    pub key: String,
    pub value: String,
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
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let db = connect(path.to_str().unwrap_or(".")).execute().await?;

        let entities = open_or_create(&db, ENTITIES_TABLE, entity_schema()).await?;
        let relations = open_or_create(&db, RELATIONS_TABLE, relation_schema()).await?;
        let journals = open_or_create(&db, JOURNALS_TABLE, journal_schema()).await?;

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

    /// Full-text search on entities, scoped by agent and optional type filter.
    pub async fn search_entities(
        &self,
        query: &str,
        agent: &str,
        entity_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EntityResult>> {
        let mut filter = format!("agent = '{}'", escape_sql(agent));
        if let Some(et) = entity_type {
            filter.push_str(&format!(" AND entity_type = '{}'", escape_sql(et)));
        }
        let batches: Vec<RecordBatch> = self
            .entities
            .query()
            .full_text_search(FullTextSearchQuery::new(query.to_owned()))
            .only_if(filter)
            .limit(limit)
            .execute()
            .await?
            .try_collect()
            .await?;

        batches_to_entities(&batches)
    }

    /// Query entities by type and agent (no FTS, returns all matching).
    pub async fn query_by_type(
        &self,
        agent: &str,
        entity_type: &str,
        limit: usize,
    ) -> Result<Vec<EntityResult>> {
        let filter = format!(
            "agent = '{}' AND entity_type = '{}'",
            escape_sql(agent),
            escape_sql(entity_type)
        );
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

    /// Look up an entity by key within an agent's scope.
    pub async fn find_entity_by_key(&self, key: &str, agent: &str) -> Result<Option<EntityResult>> {
        let filter = format!(
            "agent = '{}' AND key = '{}'",
            escape_sql(agent),
            escape_sql(key)
        );
        let batches: Vec<RecordBatch> = self
            .entities
            .query()
            .only_if(filter)
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;

        Ok(batches_to_entities(&batches)?.into_iter().next())
    }

    /// Upsert a relation (deduplicated by source+relation+target+agent).
    pub async fn upsert_relation(&self, row: &RelationRow<'_>) -> Result<()> {
        let batch = make_relation_batch(row)?;
        let schema = batch.schema();
        let batches = RecordBatchIterator::new(std::iter::once(Ok(batch)), schema);

        let mut merge = self
            .relations
            .merge_insert(&["source", "relation", "target", "agent"]);
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
        agent: &str,
        relation: Option<&str>,
        direction: Direction,
        limit: usize,
    ) -> Result<Vec<RelationResult>> {
        let limit = limit.min(CONNECTIONS_MAX);
        let cypher = build_connections_cypher(entity_id, agent, relation, direction, limit);
        let query = CypherQuery::new(&cypher)?.with_config(self.graph_config.clone());
        let batch = query
            .execute_with_namespace_arc(Arc::clone(&self.namespace), None)
            .await?;

        batch_to_relations(&batch)
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
            (
                vec!["agent"],
                Index::Bitmap(Default::default()),
                "entities agent",
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
            (
                vec!["agent"],
                Index::Bitmap(Default::default()),
                "relations agent",
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

fn entity_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("entity_type", DataType::Utf8, false),
        Field::new("key", DataType::Utf8, false),
        Field::new("value", DataType::Utf8, false),
        Field::new("agent", DataType::Utf8, false),
        Field::new("created_at", DataType::UInt64, false),
        Field::new("updated_at", DataType::UInt64, false),
    ]))
}

fn relation_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("source", DataType::Utf8, false),
        Field::new("relation", DataType::Utf8, false),
        Field::new("target", DataType::Utf8, false),
        Field::new("agent", DataType::Utf8, false),
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
    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![row.id])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![row.entity_type])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![row.key])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![row.value])) as Arc<dyn Array>,
            Arc::new(StringArray::from(vec![row.agent])) as Arc<dyn Array>,
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
            Arc::new(StringArray::from(vec![row.agent])) as Arc<dyn Array>,
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
        for i in 0..batch.num_rows() {
            results.push(EntityResult {
                id: ids.value(i).to_string(),
                entity_type: types.value(i).to_string(),
                key: keys.value(i).to_string(),
                value: values.value(i).to_string(),
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
    agent: &str,
    relation: Option<&str>,
    direction: Direction,
    limit: usize,
) -> String {
    let eid = escape_cypher(entity_id);
    let ag = escape_cypher(agent);

    let rel_type = relation
        .map(|r| format!(":`{}`", escape_cypher_ident(r)))
        .unwrap_or_default();

    let (pattern, agent_filter) = match direction {
        Direction::Outgoing => (
            format!("(e:entities {{id: '{eid}'}})-[r:relations{rel_type}]->(t:entities)"),
            format!("r.agent = '{ag}'"),
        ),
        Direction::Incoming => (
            format!("(e:entities)<-[r:relations{rel_type}]-(s:entities {{id: '{eid}'}})"),
            format!("r.agent = '{ag}'"),
        ),
        Direction::Both => (
            format!("(e:entities)-[r:relations{rel_type}]-(o:entities {{id: '{eid}'}})"),
            format!("r.agent = '{ag}'"),
        ),
    };

    format!(
        "MATCH {pattern} WHERE {agent_filter} RETURN r.source, r.relation, r.target LIMIT {limit}"
    )
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
