// Deny-by-default publication checks: fixture values must be recognizably fictional
// or explicitly approved protocol literals. This is not a general-purpose sanitizer.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const manifest = JSON.parse(readFileSync(join(__dirname, 'twg/manifest.json'), 'utf8'));
const protocol = new Set(('v2 doc paragraph text heading bulletList orderedList listItem hardBreak inlineCard mention codeBlock table tableRow tableCell tableHeader strong em link code rule status media mediaSingle blockquote panel expand current page content global scrum software entity native rest json public private active pending in_progress on_track at_risk off_track completed demo').split(' '));
const commands = new Set(['jira.workitem.query', 'jira.workitem.get', 'jira.board.query', 'confluence.search.query', 'teams.query', 'projects.query', 'goals.query']);
// Audited structural schema keys, not tenant-specific field display names.
const keys = new Set(`_expandable _links accessLevel accountId accountIds accountType acquisition active aggregateprogress aggregatetimeestimate aggregatetimeoriginalestimate aggregatetimespent all ancestors apiVersion archivedResultCount ari assignee attachment attrs author avatarId avatarURI backend base boards body breadcrumbs child childTypes children color colorName command comment comments compassComponentState compassComponentVersion components container content context contributingProjects count coverage covered cql cqlQuery created createdSince creationDate creator cursor data dataclassification dateRange descendants description displayName displayUrl dueDate duedate edges email emailAddress end entityId entityType environment evidence excerpt expand extensions failed fields fixVersions friendlyLastModified full hasNextPage hasVoted hierarchyLevel history href iconCssClass id includeContributingProjects includeDescription includeInferred includeLinkedGoals includeParentGoal includesYou input inward inwardIssue isArchived isCurrentUserMemberOfTeam isLast isNumberColumnEnabled isPrivate isShared isVerified isVisible isWatching issueIdOrKey issuelinks issuerestriction issuerestrictions issues issuetype items jql jsdPublic key label labels language languageCode lastModified lastViewed latestUpdateDate latestUserUpdate layout level limit linkedGoals localId location macroRenderedOutput marks matched matchedExact maxResults measurement member memberCount members meta metadata name names newState next nextCursor node nodes objectId ok omitted operations order organizationId outward outwardIssue owner page pageInfo panelType parent parentGoal picture priority profileUrl progress project projectCategory projectId projectKey projectName projectTypeKey provenance query reason remoteLinks reporter request requested resolution resolutiondate resolvedStatusFilter resourceClass resourceType restrictions resultCount resultGlobalContainer results returned role rootAccountId rootEmail rootName scope score searchDuration security self shortName shouldDisplay simplified site size smallAvatarImageUrl source sourceMode space sqliteFile start startAt startDate state status statusAll statusCategory statuscategorychangedate subType subtask subtasks subtype succeeded summary summaryText tag tags targetDate team text timeZone timeestimate timeoriginalestimate timespent timestamp timetracking tinyui title total totalSize tql truncated truncation type typeId updateAuthor updated updatedSince url userAccountId userId value version versions votes watchCount watches webui what why worklog worklogs workratio workspaceId wrap`.split(/\s+/));
const paragraph = 'Demo service migration improves request routing and cache reliability. The synthetic team validates retry handling, monitors latency, and documents rollback steps before the next release. ';

function safeString(value, key) {
  if (value === '' || protocol.has(value)) return true;
  if (key === 'command') return commands.has(value);
  if (/^(?:demo-\d{6}|DEMO-\d{4}|Demo Person \d{4})$/.test(value)) return true;
  if (/^https:\/\/demo\.example\.invalid\/resource\/\d{6}$/.test(value)) return true;
  if (/^person-\d{4}@example\.invalid$/.test(value)) return true;
  if (/^2025-\d{2}-\d{2}T12:00:00\.000Z$/.test(value)) return true;
  if (/^(Cache reliability|Search indexing|Release automation|Queue capacity|API pagination|Service observability|Storage migration|Access controls|Build performance|Incident response) \d+$/.test(value)) return true;
  return value === paragraph.repeat(Math.ceil(value.length / paragraph.length)).slice(0, value.length);
}

function audit(value, path = [], key = '') {
  if (typeof value === 'string') {
    assert.ok(safeString(value, key), `Unapproved string at ${path.join('.')}`);
  } else if (typeof value === 'number') {
    assert.ok(Number.isInteger(value) && (value === 0 || value === 1 || (value >= 1001 && value < 10000)), `Non-synthetic number at ${path.join('.')}`);
  } else if (Array.isArray(value)) {
    value.forEach((v, i) => audit(v, [...path, i], key));
  } else if (value && typeof value === 'object') {
    for (const [k, v] of Object.entries(value)) {
      assert.ok(keys.has(k) || /^(?:customfield_\d{5}|demo_property_\d{6})$/.test(k), `Unapproved property at ${[...path, k].join('.')}`);
      audit(v, [...path, k], k);
    }
  }
}

for (const entry of manifest) {
  test(`${entry.id}: every nested key and scalar is publication-safe`, () => {
    audit(JSON.parse(readFileSync(join(__dirname, 'twg', `${entry.id}.json`), 'utf8')));
  });
}
test('privacy guard rejects internal-looking and arbitrary content', () => {
  for (const value of ['https://company.atlassian.net/wiki/page', 'https://demo.example.invalid.evil.com/resource/000001', 'person@company.com', '/Users/someone/private', 'internal roadmap text', 'demo-original-identifier']) {
    assert.equal(safeString(value, 'text'), false);
  }
  assert.throws(() => audit({ 'Customer Secret': 'demo' }));
  assert.throws(() => audit({ text: 'demo-000001\nsecret' }));
  assert.throws(() => audit({ id: 123456789 }));
});
