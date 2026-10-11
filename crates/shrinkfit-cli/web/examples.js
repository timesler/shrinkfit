// Example inputs for the playground. Everything is generated deterministically
// (seeded PRNG) so results are reproducible between reloads. Each example
// returns { name, desc, text, max_chars? } where `text` is JSON or YAML.

function rng(seed) {
  let s = seed >>> 0 || 1;
  const next = () => { s ^= s << 13; s >>>= 0; s ^= s >>> 17; s ^= s << 5; s >>>= 0; return s / 4294967296; };
  next.int = (n) => Math.floor(next() * n);
  next.pick = (a) => a[next.int(a.length)];
  return next;
}

const WORDS = ("service request latency cache queue deploy rollback timeout retry shard index cluster node pod ingress " +
  "token session policy account billing invoice customer region failover replica snapshot restore migration schema " +
  "query planner optimizer metric alert dashboard runbook incident owner escalation pipeline artifact release branch " +
  "review approval compliance audit encryption rotation certificate gateway throttle quota budget forecast capacity " +
  "workload autoscale rollout canary feature flag experiment cohort funnel retention").split(" ");

function sentence(r, n) {
  const w = Array.from({ length: n ?? 8 + r.int(14) }, () => r.pick(WORDS));
  w[0] = w[0][0].toUpperCase() + w[0].slice(1);
  return w.join(" ") + ".";
}
function paragraph(r, sentences) {
  return Array.from({ length: sentences ?? 3 + r.int(4) }, () => sentence(r)).join(" ");
}
function title(r) {
  return Array.from({ length: 3 + r.int(4) }, () => r.pick(WORDS)).map((w) => w[0].toUpperCase() + w.slice(1)).join(" ");
}
const iso = (r, y = 2024) => `${y}-${String(1 + r.int(12)).padStart(2, "0")}-${String(1 + r.int(28)).padStart(2, "0")}T${String(r.int(24)).padStart(2, "0")}:${String(r.int(60)).padStart(2, "0")}:00.000Z`;
const hex = (r, n) => Array.from({ length: n }, () => r.int(16).toString(16)).join("");

function searchResponse() {
  const r = rng(1);
  const results = Array.from({ length: 40 }, (_, i) => {
    const id = `doc_${hex(r, 12)}`;
    const snippets = Array.from({ length: 3 + r.int(4) }, (_, j) => {
      const text = paragraph(r, 2 + r.int(3));
      const hl = r.pick(WORDS);
      return { field: r.pick(["body", "title", "summary", "comments"]), rank: j + 1, score: +r().toFixed(4), text, highlights: [{ term: hl, start: r.int(40), end: 40 + r.int(40) }] };
    });
    return {
      id, rank: i + 1, score: +(12 - i * 0.21 + r()).toFixed(4),
      url: `https://docs.example.com/${r.pick(["wiki", "kb", "blog", "ref"])}/${hex(r, 8)}`,
      title: title(r),
      source: { type: r.pick(["confluence", "github", "drive", "slack", "zendesk"]), space: r.pick(["ENG", "OPS", "SEC", "DATA"]), connector_id: `conn_${hex(r, 6)}` },
      author: { id: `u_${hex(r, 8)}`, name: title(r).split(" ").slice(0, 2).join(" "), email: `${r.pick(WORDS)}.${r.pick(WORDS)}@example.com` },
      created_at: iso(r, 2023), updated_at: iso(r, 2024),
      tags: Array.from({ length: 2 + r.int(5) }, () => r.pick(WORDS)),
      metadata: { language: "en", word_count: 300 + r.int(9000), permissions: { visibility: r.pick(["org", "team", "private"]), groups: ["eng", "ops"] }, content_type: "text/html", version: 1 + r.int(40), labels: { reviewed: r() > 0.5, archived: false } },
      snippets,
      passages: Array.from({ length: 2 }, () => ({ offset: r.int(50000), length: 600, text: paragraph(r, 6) })),
    };
  });
  return {
    name: "Search response",
    desc: "40 ranked results, each with rich metadata, snippets and passages",
    max_chars: 10000,
    text: JSON.stringify({ query: "capacity planning for autoscale rollout", total_results: 18342, took_ms: 87, page: 1, page_size: 40, next_page_token: hex(r, 48), facets: { source: { confluence: 9120, github: 4311, drive: 2890, slack: 1500, zendesk: 521 } }, results }),
  };
}

function confluencePage() {
  const r = rng(2);
  let body = `<h1>${title(r)}</h1><p>${paragraph(r, 5)}</p>`;
  body += `<ac:structured-macro ac:name="toc" ac:schema-version="1"><ac:parameter ac:name="maxLevel">3</ac:parameter></ac:structured-macro>`;
  for (let s = 0; s < 14; s++) {
    body += `<h2>${title(r)}</h2>`;
    for (let p = 0; p < 3; p++) body += `<p>${paragraph(r)}</p>`;
    body += `<ul>${Array.from({ length: 5 }, () => `<li>${sentence(r)}</li>`).join("")}</ul>`;
    if (s % 3 === 0) body += `<ac:structured-macro ac:name="code" ac:schema-version="1"><ac:parameter ac:name="language">yaml</ac:parameter><ac:plain-text-body><![CDATA[replicas: ${1 + r.int(9)}\nresources:\n  limits:\n    cpu: "${1 + r.int(4)}"\n    memory: ${r.pick([512, 1024, 2048])}Mi\nautoscaling:\n  enabled: true\n  maxReplicas: ${10 + r.int(40)}]]></ac:plain-text-body></ac:structured-macro>`;
    if (s % 4 === 1) body += `<table><tbody><tr><th>Metric</th><th>Target</th><th>Owner</th></tr>${Array.from({ length: 6 }, () => `<tr><td>${r.pick(WORDS)}</td><td>${r.int(1000)}</td><td><ri:user ri:account-id="${hex(r, 24)}"/></td></tr>`).join("")}</tbody></table>`;
  }
  const user = () => ({ type: "known", accountId: hex(r, 24), accountType: "atlassian", email: "", publicName: title(r).split(" ").slice(0, 2).join(" "), profilePicture: { path: `/wiki/aa-avatar/${hex(r, 24)}`, width: 48, height: 48, isDefault: false }, displayName: title(r).split(" ").slice(0, 2).join(" "), isExternalCollaborator: false });
  const page = {
    id: "5120948611", type: "page", status: "current", title: "Capacity Planning Runbook",
    space: { id: 9830411, key: "ENG", name: "Engineering", type: "global", status: "current", _links: { webui: "/spaces/ENG", self: "https://example.atlassian.net/wiki/rest/api/space/ENG" } },
    history: { latest: true, createdBy: user(), createdDate: iso(r, 2022), _links: { self: "https://example.atlassian.net/wiki/rest/api/content/5120948611/history" } },
    version: { by: user(), when: iso(r, 2024), friendlyWhen: "3 days ago", message: "Update scaling targets", number: 47, minorEdit: false, confRev: "confluence$content$5120948611.52", contentTypeModified: false },
    ancestors: Array.from({ length: 3 }, (_, i) => ({ id: String(5100000000 + i), type: "page", status: "current", title: title(r), _links: { webui: `/spaces/ENG/pages/${5100000000 + i}`, self: `https://example.atlassian.net/wiki/rest/api/content/${5100000000 + i}` } })),
    body: { storage: { value: body, representation: "storage", embeddedContent: [], _expandable: { content: "/rest/api/content/5120948611" } }, _expandable: { editor: "", atlas_doc_format: "", view: "", export_view: "", styled_view: "", dynamic: "", editor2: "", anonymous_export_view: "" } },
    extensions: { position: 12 },
    metadata: { labels: { results: ["runbook", "capacity", "sre"].map((n, i) => ({ prefix: "global", name: n, id: String(900 + i), label: n })), start: 0, limit: 200, size: 3 } },
    macroRenderedOutput: {},
    _expandable: { container: "/rest/api/space/ENG", operations: "", children: "/rest/api/content/5120948611/child", restrictions: "/rest/api/content/5120948611/restriction/byOperation", descendants: "/rest/api/content/5120948611/descendant" },
    _links: { webui: "/spaces/ENG/pages/5120948611/Capacity+Planning+Runbook", edit: "/pages/resumedraft.action?draftId=5120948611", tinyui: "/x/AwBp", self: "https://example.atlassian.net/wiki/rest/api/content/5120948611", base: "https://example.atlassian.net/wiki", context: "/wiki" },
  };
  return { name: "Confluence page", desc: "Content API response dominated by a large storage-format (XHTML) body", max_chars: 10000, text: JSON.stringify(page) };
}

function adf(r, paras) {
  const text = (t) => ({ type: "text", text: t });
  const content = [];
  for (let i = 0; i < paras; i++) {
    content.push({ type: "paragraph", content: [text(paragraph(r, 3)), { type: "text", text: r.pick(WORDS), marks: [{ type: "strong" }] }, text(" " + sentence(r))] });
    if (i % 3 === 0) content.push({ type: "bulletList", content: Array.from({ length: 3 }, () => ({ type: "listItem", content: [{ type: "paragraph", content: [text(sentence(r))] }] })) });
  }
  return { version: 1, type: "doc", content };
}

function jiraIssue() {
  const r = rng(3);
  const person = () => ({ accountId: hex(r, 24), displayName: title(r).split(" ").slice(0, 2).join(" "), active: true, accountType: "atlassian", avatarUrls: { "48x48": `https://avatar.example.com/${hex(r, 10)}/48`, "24x24": `https://avatar.example.com/${hex(r, 10)}/24` } });
  const issue = {
    expand: "renderedFields,names,schema,operations,editmeta,changelog,versionedRepresentations",
    id: "104233", key: "OPS-4821", self: "https://example.atlassian.net/rest/api/3/issue/104233",
    fields: {
      summary: "Autoscaler oscillates under bursty load",
      issuetype: { id: "10004", name: "Bug", subtask: false, iconUrl: "https://example.atlassian.net/images/bug.svg" },
      status: { id: "3", name: "In Progress", statusCategory: { id: 4, key: "indeterminate", colorName: "yellow", name: "In Progress" } },
      priority: { id: "2", name: "High" }, labels: ["autoscale", "sre", "p1"], reporter: person(), assignee: person(),
      created: iso(r), updated: iso(r), components: [{ id: "10010", name: "autoscaler" }], fixVersions: [],
      description: adf(r, 12),
      comment: { total: 24, maxResults: 24, startAt: 0, comments: Array.from({ length: 24 }, () => ({ id: String(100000 + r.int(99999)), author: person(), body: adf(r, 2 + r.int(4)), created: iso(r), updated: iso(r), visibility: null })) },
      attachment: Array.from({ length: 4 }, () => ({ id: String(r.int(99999)), filename: `${r.pick(WORDS)}-${r.int(99)}.png`, size: r.int(900000), mimeType: "image/png", content: `https://example.atlassian.net/secure/attachment/${hex(r, 8)}` })),
    },
  };
  return { name: "Jira issue", desc: "Issue with an ADF (nested JSON) description and 24 ADF comments", max_chars: 10000, text: JSON.stringify(issue) };
}

function githubIssues() {
  const r = rng(4);
  const user = () => ({ login: `${r.pick(WORDS)}-${r.int(99)}`, id: r.int(9e6), type: "User", site_admin: false, avatar_url: `https://avatars.example.com/u/${r.int(9e6)}` });
  const items = Array.from({ length: 60 }, (_, i) => ({
    url: `https://api.example.com/repos/acme/app/issues/${i + 1}`, number: i + 1, title: title(r), state: r.pick(["open", "closed"]),
    user: user(), labels: Array.from({ length: r.int(4) }, () => ({ id: r.int(1e8), name: r.pick(WORDS), color: hex(r, 6), default: false })),
    assignees: r() > 0.5 ? [user()] : [], comments: r.int(30), created_at: iso(r), updated_at: iso(r),
    body: Array.from({ length: 1 + r.int(5) }, () => paragraph(r, 3 + r.int(5))).join("\n\n"),
    reactions: { total_count: r.int(20), "+1": r.int(10), heart: r.int(5) },
  }));
  return { name: "Issue list", desc: "60 homogeneous GitHub-style issues with markdown bodies", max_chars: 8000, text: JSON.stringify(items) };
}

function buildLog() {
  const r = rng(5);
  const levels = ["INFO", "INFO", "INFO", "DEBUG", "WARN"];
  const log = (n) => Array.from({ length: n }, (_, i) => `2024-05-02T10:${String(i % 60).padStart(2, "0")}:${String(r.int(60)).padStart(2, "0")}Z ${r.pick(levels)} [${r.pick(WORDS)}] ${sentence(r, 6 + r.int(8))}`).join("\n");
  const steps = ["checkout", "install", "compile", "unit tests", "integration tests", "package", "deploy"].map((name, i) => ({
    name, number: i + 1, status: "completed", conclusion: i === 4 ? "failure" : "success", started_at: iso(r), completed_at: iso(r),
    log: log(i === 4 ? 900 : 40 + r.int(200)),
  }));
  return { name: "CI build log", desc: "A job whose steps carry long multi-line log strings (tests line-level previews)", max_chars: 6000, text: JSON.stringify({ job: "build-and-test", run_id: 8812340011, runner: { os: "linux", arch: "x64" }, steps }) };
}

function k8sYaml() {
  const r = rng(6);
  const docs = Array.from({ length: 8 }, (_, i) => {
    const name = `${r.pick(WORDS)}-${r.pick(WORDS)}`;
    return [
      "apiVersion: apps/v1", "kind: Deployment", "metadata:", `  name: ${name}`, "  namespace: production", "  labels:", `    app: ${name}`, `    tier: ${r.pick(["web", "api", "worker"])}`,
      "spec:", `  replicas: ${1 + r.int(8)}`, "  selector:", "    matchLabels:", `      app: ${name}`, "  template:", "    metadata:", "      labels:", `        app: ${name}`,
      "    spec:", "      containers:", `        - name: ${name}`, `          image: registry.example.com/${name}:${1 + r.int(9)}.${r.int(20)}.${r.int(50)}`,
      "          ports:", `            - containerPort: ${8000 + r.int(900)}`, "          env:",
      ...Array.from({ length: 8 }, () => [`            - name: ${r.pick(WORDS).toUpperCase()}_${r.pick(WORDS).toUpperCase()}`, `              value: "${hex(r, 16)}"`]).flat(),
      "          resources:", "            limits:", `              cpu: "${1 + r.int(4)}"`, `              memory: ${r.pick([512, 1024, 2048])}Mi`,
      "          livenessProbe:", "            httpGet:", "              path: /healthz", `              port: ${8000 + r.int(900)}`,
    ].join("\n");
  });
  return { name: "Kubernetes manifests (YAML)", desc: "Multi-document YAML; exercises YAML auto-detection (documents become an array)", max_chars: 5000, text: docs.join("\n---\n") + "\n" };
}

function dbRows() {
  const r = rng(7);
  const rows = Array.from({ length: 800 }, (_, i) => ({ id: i + 1, user_id: r.int(50000), event: r.pick(["view", "click", "purchase", "refund"]), amount_cents: r.int(100000), currency: "USD", ip: `10.${r.int(255)}.${r.int(255)}.${r.int(255)}`, ts: iso(r), country: r.pick(["US", "DE", "JP", "BR", "IN"]), flags: { new_user: r() > 0.8, mobile: r() > 0.5 } }));
  return { name: "Database rows", desc: "800 uniform analytics rows: wide, flat, no long strings", max_chars: 4000, text: JSON.stringify({ query: "SELECT * FROM events WHERE ts > now() - interval '1 day'", row_count: rows.length, columns: Object.keys(rows[0]), rows }) };
}

function auditEvents() {
  const r = rng(8);
  const actor = () => ({ id: hex(r, 10), name: title(r).split(" ")[0], ip: `192.168.${r.int(255)}.${r.int(255)}` });
  const kinds = [
    () => ({ type: "login", actor: actor(), method: r.pick(["password", "sso", "token"]), success: r() > 0.1, user_agent: "Mozilla/5.0 " + hex(r, 20) }),
    () => ({ type: "permission.change", actor: actor(), target: { kind: "role", name: r.pick(WORDS) }, diff: { added: [r.pick(WORDS)], removed: [r.pick(WORDS)] }, reason: paragraph(r, 2) }),
    () => ({ type: "deploy", actor: actor(), service: r.pick(WORDS), version: `${r.int(9)}.${r.int(30)}.${r.int(99)}`, environment: r.pick(["staging", "prod"]), changelog: paragraph(r, 5) }),
    () => ({ type: "export", actor: actor(), dataset: r.pick(WORDS), rows: r.int(1e6), destination: { kind: "s3", bucket: `bkt-${hex(r, 6)}`, key: `exports/${hex(r, 10)}.csv` } }),
  ];
  const events = Array.from({ length: 150 }, (_, i) => ({ seq: i, at: iso(r), ...r.pick(kinds)() }));
  return { name: "Audit events", desc: "Heterogeneous event stream: several shapes in one array (tests shape-aware sampling)", max_chars: 6000, text: JSON.stringify({ tenant: "acme", events }) };
}

const EXAMPLES = [searchResponse, confluencePage, jiraIssue, githubIssues, buildLog, k8sYaml, dbRows, auditEvents];
