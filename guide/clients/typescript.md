# Node.js / TypeScript

pgRDF is plain SQL, so any PostgreSQL client for Node works. Examples
below use the Docker setup from the [install guide](../01-install.md).

## node-postgres (`pg`)

```bash
npm install pg
npm install --save-dev @types/pg
```

```ts
import { Client } from 'pg';

const client = new Client({ connectionString: 'postgresql://postgres:pgrdf@localhost:5432/postgres' });
await client.connect();

await client.query('CREATE EXTENSION IF NOT EXISTS pgrdf');

// create a graph and load Turtle
const { rows: [{ add_graph: graphId }] } = await client.query<{ add_graph: string }>(
  'SELECT pgrdf.add_graph($1)', ['http://example.org/people']);

await client.query('SELECT pgrdf.parse_turtle($1, $2)', [
  `@prefix ex: <http://example.org/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice foaf:name "Alice" ; foaf:knows ex:bob .
   ex:bob   foaf:name "Bob" .`,
  graphId,
]);

// SPARQL: each row's `sparql` column is the parsed JSON binding
type Binding = { who: string; friend: string };
const { rows } = await client.query<{ sparql: Binding }>(
  'SELECT sparql FROM pgrdf.sparql($1)',
  [`PREFIX foaf: <http://xmlns.com/foaf/0.1/>
    SELECT ?who ?friend WHERE { ?a foaf:name ?who ; foaf:knows ?b . ?b foaf:name ?friend }`],
);
for (const { sparql } of rows) console.log(sparql.who, '→', sparql.friend);

await client.end();
```

`BIGINT` values such as graph ids arrive as strings in `pg`; pass them
back as-is. SPARQL values are always strings; convert numbers yourself.

### Handling refusals

```ts
try {
  await client.query('SELECT pgrdf.clear_graph($1)', ['http://example.org/people']);
} catch (e: any) {
  switch (e.code) {
    case '55P03': /* graph is locked — e.message names the unlock */ break;
    case '42704': /* no such graph */ break;
    case '0A000': /* unsupported construct */ break;
    default: throw e;
  }
}
```

### Was the answer complete?

Run this on the **same connection** right after the query (use a
`Client`, or check out one client from a pool):

```ts
const { rows: [{ last_call_stats: s }] } = await client.query('SELECT pgrdf.last_call_stats()');
const complete = s.path_depth_truncations === 0 && s.filter_clauses_dropped === 0;
```

### Large result sets

Stream with `pg-cursor`:

```ts
import Cursor from 'pg-cursor';

const cursor = client.query(new Cursor('SELECT sparql FROM pgrdf.sparql($1)',
  ['SELECT ?s ?p ?o WHERE { ?s ?p ?o }']));
for (let batch = await cursor.read(1000); batch.length; batch = await cursor.read(1000)) {
  for (const { sparql } of batch) { /* ... */ }
}
await cursor.close();
```

## postgres.js

```bash
npm install postgres
```

```ts
import postgres from 'postgres';
const sql = postgres('postgres://postgres:pgrdf@localhost/postgres');

const [{ add_graph: g }] = await sql`SELECT pgrdf.add_graph(${'http://example.org/people'})`;
await sql`SELECT pgrdf.parse_turtle(${'<http://example.org/a> <http://example.org/p> "x" .'}, ${g})`;

const rows = await sql<{ sparql: Record<string, string> }[]>`
  SELECT sparql FROM pgrdf.sparql(${'SELECT ?s ?o WHERE { ?s <http://example.org/p> ?o }'})`;
rows.forEach(({ sparql }) => console.log(sparql.s, sparql.o));

await sql.end();
```

## Tips

- `pgrdf.load_turtle(path, …)` reads from the **database server's**
  filesystem. For files next to your app, read them and use
  `parse_turtle`.
- SQL parameters (`$1`) protect the SQL call, not the SPARQL inside it.
  Escape user input you splice into query text.
- A long load holds its connection. Run bulk loads on a dedicated
  connection so other queries don't queue behind them.
