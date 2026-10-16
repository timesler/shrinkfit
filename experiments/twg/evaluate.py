#!/usr/bin/env python3
"""TWG structural utility evaluation; model replies cached for deterministic replay.

Build: cargo build --release -p shrinkfit --example evaluate
Run: OPENROUTER_API_KEY=... python3 experiments/twg/evaluate.py --output target/twg-baseline.json
Only fictional fixtures and their field summaries are sent to OpenRouter. No keys are
written to disk. Replies are cached by full request SHA256, never by fixture name.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / 'crates/shrinkfit-cli/web/twg'
# Predeclared task contracts: usable identity + readable title + relevant detail.
CASES = {
    'jira-search': ('data/issues', 'key', ['summary', 'status/name', 'assignee/displayName', 'priority/name']),
    'jira-all-fields': ('data/issues', 'key', ['summary', 'status/name', 'assignee/displayName', 'priority/name']),
    'jira-details': ('data/items', 'data/key', ['data/summary', 'data/status/name', 'data/assignee/displayName', 'data/priority/name']),
    'confluence-search': ('data/results', 'content/id', ['title', 'url', 'excerpt', 'lastModified']),
    'teams': ('data', 'team/id', ['team/displayName', 'team/description', 'memberCount', 'team/profileUrl']),
    'projects': ('data', 'key', ['name', 'state/label', 'owner/name', 'description/what']),
    'goals': ('data', 'key', ['name', 'status/label', 'owner/name', 'targetDate/label']),
    'boards': ('data/boards', 'id', ['name', 'type', 'location/name', 'isPrivate']),
}


def at(value, path):
    for part in path.split('/'):
        if not isinstance(value, dict): return None
        value = value.get(part)
    return value


def leaves(value, path=''):
    if isinstance(value, dict) and value:
        for key, child in value.items(): yield from leaves(child, path+'/'+key)
    elif isinstance(value, list) and value:
        for child in value: yield from leaves(child, path+'/*')
    else: yield path


def alternatives(field):
    # Public APIs often expose equivalent readable labels and machine state values.
    if field == 'title': return ['title', 'content/title']
    if field.endswith('/label'): return [field, field[:-5]+'value']
    if field == 'description/what': return [field, 'description/why']
    return [field]


def retained(source, output):
    if source is None or source == '': return None
    if output == source: return 1.0
    if isinstance(source, str) and isinstance(output, str) and '…' in output:
        prefix = output.split('…', 1)[0]
        return min(len(prefix)/max(len(source), 1), 1.0) if source.startswith(prefix) else 0.0
    return 0.0


def metrics(source, output, case):
    path, identity, fields = case
    originals = at(source, path) or []
    reduced = at(output, path) or []
    if not isinstance(reduced, list): reduced = []
    # Either a canonical ID or a human-facing key can support follow-up calls.
    # Do not penalize a scorer merely for choosing one valid identity over another.
    identities = list(dict.fromkeys([identity, identity.rsplit('/', 1)[0]+'/id' if '/' in identity else 'id']))
    index = {(field, str(at(r, field))): r for r in reduced if isinstance(r, dict) for field in identities if at(r, field) is not None}
    score, count, titles, ids = 0.0, 0, 0, 0
    for record in originals:
        if not isinstance(record, dict): continue
        out = next((index[(field, str(at(record, field)))] for field in identities if at(record, field) is not None and (field, str(at(record, field))) in index), {})
        ids += bool(out)
        titles += bool(out) and any(at(record, f) is not None and at(out, f) == at(record, f) for f in alternatives(fields[0]))
        for field in fields:
            vals = [retained(at(record, f), at(out, f)) for f in alternatives(field)]
            vals = [v for v in vals if v is not None]
            if vals:
                score += max(vals)
                count += 1
    return {'identity_recall': ids/max(len(originals),1), 'usable_record_recall': titles/max(len(originals),1), 'task_value_recall': score/max(count,1)}


class Adapter:
    def __init__(self, binary):
        self.proc = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    def call(self, action, text, options):
        self.proc.stdin.write(json.dumps({'action':action,'input':text,'options':options})+'\n'); self.proc.stdin.flush()
        line=self.proc.stdout.readline()
        if not line: raise RuntimeError('evaluation adapter exited')
        return json.loads(line)
    def close(self):
        self.proc.stdin.close(); self.proc.wait(timeout=10)


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=ROOT/'target/release/examples/evaluate')
    parser.add_argument('--cache', type=Path, default=ROOT/'target/twg-model-cache')
    parser.add_argument('--offline', action='store_true')
    parser.add_argument('--variants', nargs='+', choices=['original', 'reversed', 'metadata-heavy'], default=['original'])
    args=parser.parse_args(); args.cache.mkdir(parents=True,exist_ok=True)
    adapter=Adapter(args.binary)
    result={'method':'synthetic structural contracts; equal-weight case-budget observations; JEV request replay; not a semantic benchmark','cases':[],'runs':[]}
    try:
        entries = json.loads((FIXTURES/'manifest.json').read_text())
        def reverse_keys(value):
            if isinstance(value, dict): return {k:reverse_keys(v) for k,v in reversed(list(value.items()))}
            if isinstance(value, list): return [reverse_keys(v) for v in value]
            return value
        for entry, variant in [(entry, variant) for variant in args.variants for entry in entries]:
            slug=entry['id']; text=(FIXTURES/(slug+'.json')).read_text(); source=json.loads(text)
            if variant == 'reversed':
                source = reverse_keys(source)
            elif variant == 'metadata-heavy':
                source = {'diagnostics': {'trace_'+str(i): 'Synthetic diagnostic detail. '*50 for i in range(40)}, **source}
            if variant != 'original': text = json.dumps(source, indent=2)
            opts={'max_chars':1000,'array_strategy':'condense','jev':{'api_key':'evaluation-request-only','cache':False,'tool':source['command']}}
            request=adapter.call('request',text,opts)
            request_hash=hashlib.sha256(json.dumps(request,sort_keys=True).encode()).hexdigest()
            file=args.cache/(request_hash+'.json'); live_ms=None
            if file.exists(): saved=json.loads(file.read_text())
            else:
                if args.offline: raise RuntimeError('missing cached model reply for '+slug)
                key=os.environ.get('OPENROUTER_API_KEY')
                if not key: raise RuntimeError('OPENROUTER_API_KEY is required for uncached requests')
                req=urllib.request.Request('https://openrouter.ai/api/alpha/decisions',data=json.dumps(request).encode(),headers={'Authorization':'Bearer '+key,'Content-Type':'application/json'})
                start=time.perf_counter()
                with urllib.request.urlopen(req,timeout=30) as response: reply=json.load(response)
                live_ms=1000*(time.perf_counter()-start)
                saved={'reply':reply,'live_ms':live_ms}
                file.write_text(json.dumps(saved))
            listed=[f['path'] for f in json.loads(request['state'])['output_fields']]
            case_info={'id':slug,'variant':variant,'input_chars':len(text),'discovered_paths':len(set(leaves(source))),'scored_paths':len(listed),'request_bytes':len(json.dumps(request).encode()),'live_ms':saved['live_ms'],'request_hash':request_hash}
            result['cases'].append(case_info)
            print(slug,case_info,flush=True)
            for budget in sorted(set([2000,6000,12000,entry['max_chars']])):
                for mode in ['sample','heuristic','jev','jev-blend']:
                    options={'max_chars':budget,'array_strategy':'sample' if mode=='sample' else 'condense','jev':False}
                    if mode.startswith('jev'):
                        options['jev']={**opts['jev'],'weight':0.7 if mode=='jev-blend' else 1.0}
                        options['jev_reply']=json.dumps(saved['reply'])
                    timings=[]
                    for _ in range(3):
                        start=time.perf_counter(); shrink=adapter.call('shrink',text,options); timings.append(1000*(time.perf_counter()-start))
                    output=json.loads(shrink['output']); report=shrink['report']
                    assert report['fits'] and len(shrink['output'])<=budget, (slug,mode,budget)
                    if mode.startswith('jev'):
                        assert (report.get('model_scoring') or '').startswith('scored '), report
                    result['runs'].append({'id':slug,'variant':variant,'mode':mode,'budget':budget,'output_chars':len(shrink['output']),'replay_ms':statistics.median(timings),**metrics(source,output,CASES[slug])})
    finally: adapter.close()
    for mode in ['sample','heuristic','jev','jev-blend']:
        runs=[r for r in result['runs'] if r['mode']==mode]
        print(mode,{k:round(statistics.mean(r[k] for r in runs),4) for k in ['identity_recall','usable_record_recall','task_value_recall','replay_ms']},flush=True)
    args.output.parent.mkdir(parents=True,exist_ok=True); args.output.write_text(json.dumps(result,indent=2)+'\n')

if __name__=='__main__':main()
