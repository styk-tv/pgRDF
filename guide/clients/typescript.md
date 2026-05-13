# Node.js / TypeScript clients

pgRDF speaks plain Postgres, so any Node-side Postgres library works.
This page covers `pg` (node-postgres — the dominant choice) and a
short example with `postgres.js` (the modern compact alternative).

## node-postgres (`pg`)

```bash
npm install pg
npm install --save-dev @types/pg
```

```ts
import { Client } from 'pg';

const client = new Client({
  host: 'localhost',
  port: 5432,
  user: 'pgrdf',
  password: 'pgrdf',
  database: 'pgrdf',
});

await client.connect();

await client.query('CREATE EXTENSION IF NOT EXISTS pgrdf');

// Load a Turtle file from the server-side filesystem
const { rows: loadRows } = await client.query<{ load_turtle: number }>(
  'SELECT pgrdf.load_turtle($1, $2)',
  ['/fixtures/ontologies/foaf.ttl', 1],
);
console.log(`loaded ${loadRows[0].load_turtle} triples`);

// Parse an inline Turtle string
await client.query(
  'SELECT pgrdf.parse_turtle($1, $2)',
  [
    '@prefix ex: <http://example.com/> . ex:a ex:p ex:b .',
    2,
  ],
);

// Run a SPARQL SELECT — pgrdf.sparql returns SETOF JSONB, so each
// row's `sparql` column is the JSON object with the variable bindings.
const { rows: solutions } = await client.query<{ sparql: Record<string, string> }>(
  `SELECT * FROM pgrdf.sparql($1)`,
  [`PREFIX foaf: <http://xmlns.com/foaf/0.1/>
    SELECT ?s ?n WHERE { ?s foaf:name ?n }`],
);
for (const row of solutions) {
  console.log(`${row.sparql.s} → ${row.sparql.n}`);
}

// Verbose ingest stats — JSONB → JS object automatically
const { rows: statsRows } = await client.query<{ load_turtle_verbose: any }>(
  'SELECT pgrdf.load_turtle_verbose($1, $2, $3)',
  ['/fixtures/ontologies/prov.ttl', 100, 'http://www.w3.org/ns/prov#'],
);
console.log(
  `prov.ttl: ${statsRows[0].load_turtle_verbose.triples} triples`
  + ` in ${statsRows[0].load_turtle_verbose.elapsed_ms} ms`,
);

await client.end();
```

### Connection pool

For anything beyond a one-shot script, use `pg.Pool`:

```ts
import { Pool } from 'pg';
const pool = new Pool({ connectionString: 'postgresql://pgrdf:pgrdf@localhost/pgrdf' });

const { rows } = await pool.query<{ sparql: Record<string, string> }>(
  `SELECT sparql FROM pgrdf.sparql($1) WHERE sparql->>'n' ~* '^a'`,
  [`PREFIX foaf: <http://xmlns.com/foaf/0.1/>
    SELECT ?p ?n WHERE { ?p foaf:name ?n }`],
);
```

Note the WHERE filter on the JSONB output — once `pgrdf.sparql`
returns its solutions you can post-process them with any normal
Postgres JSONB operator before they reach your application.

## postgres.js

```bash
npm install postgres
```

```ts
import postgres from 'postgres';
const sql = postgres('postgres://pgrdf:pgrdf@localhost/pgrdf');

await sql`CREATE EXTENSION IF NOT EXISTS pgrdf`;

const [{ load_turtle: n }] = await sql<[{ load_turtle: number }]>`
  SELECT pgrdf.load_turtle(${'/fixtures/ontologies/foaf.ttl'}, ${1})
`;
console.log(`loaded ${n} triples`);

// JSONB rows decode to objects directly
type Binding = Record<string, string>;
const solutions = await sql<{ sparql: Binding }[]>`
  SELECT * FROM pgrdf.sparql(${`
    PREFIX foaf: <http://xmlns.com/foaf/0.1/>
    SELECT ?s ?n WHERE { ?s foaf:name ?n }
  `})
`;
for (const { sparql } of solutions) {
  console.log(sparql.s, sparql.n);
}

await sql.end();
```

`postgres.js` is template-tag-driven so parameter binding is implicit
and SQL injection is impossible by construction.

## Streaming large result sets

`pg-cursor` lets you iterate millions of `pgrdf.sparql` rows without
buffering everything in memory:

```ts
import { Client } from 'pg';
import Cursor from 'pg-cursor';

const client = new Client({ /* ... */ });
await client.connect();

const cursor = client.query(new Cursor(
  `SELECT sparql FROM pgrdf.sparql($1)`,
  [`SELECT ?s ?p ?o WHERE { ?s ?p ?o }`],
));

while (true) {
  const rows = await new Promise<any[]>((resolve, reject) => {
    cursor.read(1000, (err, rows) => (err ? reject(err) : resolve(rows)));
  });
  if (rows.length === 0) break;
  for (const row of rows) {
    // process row.sparql.{var: value}
  }
}

await cursor.close();
await client.end();
```

## Type narrowing

`pgrdf.sparql` returns `SETOF JSONB` where the keys vary by query. If
you have a fixed query shape, narrow the type:

```ts
type FoafBinding = { s: string; n: string };
type FoafRow     = { sparql: FoafBinding };

const { rows } = await client.query<FoafRow>(
  `SELECT * FROM pgrdf.sparql($1)`,
  [`PREFIX foaf: <http://xmlns.com/foaf/0.1/>
    SELECT ?s ?n WHERE { ?s foaf:name ?n }`],
);
// rows[0].sparql.s and .n are now typed
```

For dynamic queries, use `Record<string, string>` and validate keys
against the SELECT-clause variable list (you can extract that list
upfront via `pgrdf.sparql_parse`).

## Caveats

- pgRDF parses Turtle strictly through `oxttl`. Any TTL that fails
  to load is genuinely off-spec — don't silently retry with a
  "lenient" client; fix the source.
- `pgrdf.sparql` searches every graph today (no `GRAPH { … }` scope
  in v0.2). Phase 3.
- Set `search_path = pgrdf, public;` per connection if you want to
  drop the `pgrdf.` prefix on every call.
- Heavy `load_turtle` calls hold the SPI connection for the duration
  of the parse. Use a separate connection (or `pg.Pool`) for the
  ingest job so other queries don't queue behind it.
