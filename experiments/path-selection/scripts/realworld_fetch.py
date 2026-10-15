"""Fetch real responses from public JSON APIs (no keys), to test shrinkfit and Jev on data that is not from MCP-Atlas.

Writes work/realworld_pool.json in the same shape as work/pool_all.json: a list of
{"row", "tool", "args", "text"} where text is compact JSON. Responses under 3,000 characters are skipped.
APIs change over time, so a re-run will give different data; the labels in data/realworld_* belong to the run they were made on.

Run from experiments/path-selection:  python scripts/realworld_fetch.py
"""
import gzip
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request

UA = "shrinkfit-research/0.4 (https://github.com/timesler/shrinkfit)"
Q = urllib.parse.quote


def u(base, **params):
    return base + "?" + urllib.parse.urlencode(params)


# (tool name as an MCP server would expose it, the call's arguments, url)
REQUESTS = [
    ("github_search_repositories", {"query": "language:rust stars:>5000"}, u("https://api.github.com/search/repositories", q="language:rust stars:>5000", per_page=20)),
    ("github_search_repositories", {"query": "topic:mcp"}, u("https://api.github.com/search/repositories", q="topic:mcp", per_page=15)),
    ("github_list_issues", {"owner": "rust-lang", "repo": "rust", "state": "open"}, u("https://api.github.com/repos/rust-lang/rust/issues", state="open", per_page=15)),
    ("github_list_issues", {"owner": "microsoft", "repo": "vscode", "labels": "bug"}, u("https://api.github.com/repos/microsoft/vscode/issues", labels="bug", per_page=15)),
    ("github_list_commits", {"owner": "torvalds", "repo": "linux"}, u("https://api.github.com/repos/torvalds/linux/commits", per_page=12)),
    ("github_list_pull_requests", {"owner": "python", "repo": "cpython", "state": "open"}, u("https://api.github.com/repos/python/cpython/pulls", state="open", per_page=10)),
    ("github_list_releases", {"owner": "denoland", "repo": "deno"}, u("https://api.github.com/repos/denoland/deno/releases", per_page=8)),
    ("hn_search", {"query": "rust"}, u("https://hn.algolia.com/api/v1/search", query="rust", hitsPerPage=25)),
    ("hn_search_by_date", {"query": "llm agents"}, u("https://hn.algolia.com/api/v1/search_by_date", query="llm agents", tags="story", hitsPerPage=25)),
    ("npm_search", {"text": "json schema"}, u("https://registry.npmjs.org/-/v1/search", text="json schema", size=20)),
    ("npm_search", {"text": "http client"}, u("https://registry.npmjs.org/-/v1/search", text="http client", size=20)),
    ("pypi_get_package", {"name": "requests"}, "https://pypi.org/pypi/requests/json"),
    ("pypi_get_package", {"name": "fastapi"}, "https://pypi.org/pypi/fastapi/json"),
    ("crates_search", {"q": "json"}, u("https://crates.io/api/v1/crates", q="json", per_page=15)),
    ("crates_get_crate", {"name": "serde"}, "https://crates.io/api/v1/crates/serde"),
    ("weather_forecast", {"latitude": 52.52, "longitude": 13.41, "hourly": ["temperature_2m", "precipitation"]}, u("https://api.open-meteo.com/v1/forecast", latitude=52.52, longitude=13.41, hourly="temperature_2m,precipitation,wind_speed_10m", forecast_days=3)),
    ("weather_forecast", {"latitude": 40.71, "longitude": -74.0, "daily": ["temperature_2m_max"]}, u("https://api.open-meteo.com/v1/forecast", latitude=40.71, longitude=-74.0, daily="temperature_2m_max,temperature_2m_min,precipitation_sum", current="temperature_2m", forecast_days=7)),
    ("countries_by_region", {"region": "europe"}, "https://restcountries.com/v3.1/region/europe"),
    ("countries_by_region", {"region": "oceania"}, "https://restcountries.com/v3.1/region/oceania"),
    ("pokeapi_get_pokemon", {"name": "pikachu"}, "https://pokeapi.co/api/v2/pokemon/pikachu"),
    ("pokeapi_list_pokemon", {"limit": 40}, u("https://pokeapi.co/api/v2/pokemon", limit=40)),
    ("wikipedia_search", {"query": "rust programming language"}, u("https://en.wikipedia.org/w/api.php", action="query", list="search", srsearch="rust programming language", format="json", srlimit=20)),
    ("wikipedia_get_page_summary", {"title": "Alan_Turing"}, "https://en.wikipedia.org/api/rest_v1/page/summary/Alan_Turing"),
    ("osm_geocode", {"query": "cafe Berlin"}, u("https://nominatim.openstreetmap.org/search", q="cafe Berlin", format="jsonv2", addressdetails=1, limit=15)),
    ("osm_geocode", {"query": "museum Paris"}, u("https://nominatim.openstreetmap.org/search", q="museum Paris", format="jsonv2", addressdetails=1, extratags=1, limit=15)),
    ("stackoverflow_search_questions", {"tagged": "python", "sort": "votes"}, u("https://api.stackexchange.com/2.3/questions", order="desc", sort="votes", tagged="python", site="stackoverflow", pagesize=15)),
    ("stackoverflow_search_questions", {"tagged": "rust", "sort": "activity"}, u("https://api.stackexchange.com/2.3/questions", order="desc", sort="activity", tagged="rust", site="stackoverflow", pagesize=15)),
    ("crossref_search_works", {"query": "large language models"}, u("https://api.crossref.org/works", query="large language models", rows=10)),
    ("crossref_search_works", {"query": "climate"}, u("https://api.crossref.org/works", query="climate", rows=10)),
    ("openalex_search_works", {"search": "agents"}, u("https://api.openalex.org/works", search="agents", **{"per-page": 10})),
    ("usgs_earthquakes", {"period": "day"}, "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/all_day.geojson"),
    ("placeholder_list_posts", {}, "https://jsonplaceholder.typicode.com/posts"),
    ("placeholder_list_comments", {"postId": 1}, "https://jsonplaceholder.typicode.com/comments"),
    ("dockerhub_list_repositories", {"namespace": "library"}, u("https://hub.docker.com/v2/repositories/library/", page_size=20)),
    ("gitlab_list_projects", {"order_by": "star_count"}, u("https://gitlab.com/api/v4/projects", order_by="star_count", per_page=12)),
    ("huggingface_list_models", {"sort": "downloads"}, u("https://huggingface.co/api/models", sort="downloads", limit=20)),
    ("jira_search_issues", {"jql": "project = JRASERVER ORDER BY created DESC"}, u("https://jira.atlassian.com/rest/api/2/search", jql="project = JRASERVER ORDER BY created DESC", maxResults=15)),
    ("jira_search_issues", {"jql": "project = CONFSERVER AND status = Open"}, u("https://jira.atlassian.com/rest/api/2/search", jql="project = CONFSERVER AND status = Open ORDER BY votes DESC", maxResults=12)),
    ("jira_get_issue", {"issueIdOrKey": "JRASERVER-1000"}, "https://jira.atlassian.com/rest/api/2/issue/JRASERVER-1000"),
    ("confluence_search", {"cql": "type = page AND text ~ \"rest api\""}, u("https://confluence.atlassian.com/rest/api/search", cql='type = page AND text ~ "rest api"', limit=12)),
    ("jsonplaceholder_list_users", {}, "https://jsonplaceholder.typicode.com/users"),
    ("spacex_list_launches", {}, "https://api.spacexdata.com/v4/launches/past"),
    ("catfacts_breeds", {"limit": 30}, u("https://catfact.ninja/breeds", limit=30)),
    ("coingecko_markets", {"vs_currency": "usd"}, u("https://api.coingecko.com/api/v3/coins/markets", vs_currency="usd", order="market_cap_desc", per_page=15, page=1)),
]


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json", "Accept-Encoding": "gzip"})
    with urllib.request.urlopen(req, timeout=40) as r:
        data = r.read()
        if r.headers.get("Content-Encoding") == "gzip":
            data = gzip.decompress(data)
    return json.loads(data.decode("utf-8"))


def main():
    os.makedirs("work", exist_ok=True)
    out, skipped = [], []
    for i, (tool, args, url) in enumerate(REQUESTS):
        try:
            doc = fetch(url)
        except Exception as e:                      # noqa: BLE001 - report and carry on
            skipped.append((tool, f"{type(e).__name__}: {str(e)[:60]}"))
            continue
        text = json.dumps(doc, ensure_ascii=False, separators=(",", ":"))
        if len(text) < 3000:
            skipped.append((tool, f"only {len(text)} chars"))
            continue
        out.append({"row": i, "tool": tool, "args": json.dumps(args), "text": text})
        time.sleep(1.1)                             # be polite (Nominatim allows one request a second)
    json.dump(out, open("work/realworld_pool.json", "w", encoding="utf-8"), ensure_ascii=False)
    print(f"{len(out)} responses kept, {len(skipped)} skipped")
    for tool, why in skipped:
        print("  skipped", tool, "-", why)
    sizes = sorted(len(o["text"]) for o in out)
    print("sizes (chars): min", sizes[0], "median", sizes[len(sizes) // 2], "max", sizes[-1], "| distinct tools:", len({o["tool"] for o in out}))


main()
