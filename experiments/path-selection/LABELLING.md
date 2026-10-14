# Labelling protocol

Twelve independent Claude agents labelled `data/lab_cases.json` in batches of ~22 cases. Each case
shows the tool call, the first 1,500 characters of the response, and every unique JSON path with
two sample values. They were **not** shown the user's question, any heuristic score or any model
output. Each path gets one level:

* **2 essential**: the agent would be impaired without it. The main descriptive field(s) of each
  record (name/title/summary-like), the primary identifier or URL needed to refer back to a record,
  and the key facts the call was evidently asking for.
* **1 useful**: helps but could be dropped if space is tight. Secondary descriptive fields, status,
  category, timestamps, secondary ids/urls, pagination info.
* **0 drop**: decorative, redundant, verbose or uninformative. Image/avatar/icon URLs, styling, long
  boilerplate, duplicated copies of another field, internal metadata, API self-links, constants
  repeated on every record.

Context given to the labellers: a tool returned a very large JSON response; a truncation system must
drop paths so that an AI agent can still (a) understand at a glance what the response contains,
through a little descriptive content such as names, titles, short text, dates and key values, and
(b) make more targeted follow-up calls, which needs identifiers and URLs. The agent has more detail
available on demand, so the shortened version only needs enough to scan and refer back.

Rules: be selective (typically about a quarter of the paths are level 2); when two paths carry the
same information, give the better one its level and the duplicate 0; judge usefulness, not size (a
long text field is essential if it is the main content); a URL is only useful if it identifies or
leads to the record.

24 anchor cases were labelled by two agents (909 path pairs). Agreement: exact 87% (kappa 0.76);
essential vs not 96% (kappa 0.88); keep vs drop 90% (kappa 0.79). Anchor cases use the mean of the
two labels. The share of paths labelled essential varies by labeller from 7% to 19%.
