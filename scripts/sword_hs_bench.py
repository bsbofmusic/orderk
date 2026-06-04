#!/usr/bin/env python3
import json, os, pathlib, re, shutil, subprocess, time, urllib.request, urllib.error
from datetime import datetime, timezone

ROOT = pathlib.Path('/tmp/orderk-sword-hs-bench')
VAULT = ROOT / 'vault'
PROJECT = pathlib.Path('/home/agent/orderk')
BIN = PROJECT / 'target/debug/orderk'
DB = ROOT / 'orderk-qwen3.sqlite'
HS = 'http://127.0.0.1:8765'
BANK = 'orderk-sword-bench-' + datetime.now(timezone.utc).strftime('%Y%m%d%H%M%S')

DOCS = {
    'alpha.md': '''---\ntags: [orderk, sword, active-thinking]\n---\n# Sword Spirit Active Thinking\norderk V2 Sword Spirit should actively compare Markdown notes, use the same Hindsight-like reranker and MiniMax M3, then propose typed semantic edges for future search. Sword Spirit writes sidecar artifacts under .orderk and must not mutate raw Markdown.\n''',
    'bravo.md': '''---\ntags: [hindsight, retrieval, qwen3]\n---\n# Hindsight Retrieval Stack\nHindsight uses MiniMax M3 through an Anthropic-compatible endpoint plus Qwen3-Embedding-4B at 1024 dimensions and Qwen3-Reranker-4B. It is the benchmark stack for recall, reranking, and deeper reflection.\n''',
    'charlie.md': '''# Garden Tomato Notes\nTomatoes need steady water, sunlight, and soil drainage. This gardening note is unrelated to semantic retrieval benchmarks or Hindsight.\n''',
    'delta.md': '''---\ntags: [markdown-base, noteriv, orderk]\n---\n# External Markdown Base Boundary\nNoteriv or another Markdown-first base owns the vault and editor surface. orderk should stay a lightweight intelligence layer over plain Markdown: index, search, proposal sidecars, evaluation, and Sword Spirit digest loops.\n''',
}

QUERIES = [
    {'id':'q_sword_stack','query':'Sword Spirit should use Hindsight reranker and MiniMax M3','expected':['alpha.md','bravo.md']},
    {'id':'q_hs_qwen','query':'Which note describes Qwen3 reranker and Qwen3 embedding for Hindsight?','expected':['bravo.md']},
    {'id':'q_garden','query':'tomatoes water sunlight soil drainage','expected':['charlie.md']},
    {'id':'q_boundary','query':'external Markdown base owns vault editor while orderk writes sidecar proposals','expected':['delta.md','alpha.md']},
]


def clean():
    if ROOT.exists():
        shutil.rmtree(ROOT)
    VAULT.mkdir(parents=True)
    for name, content in DOCS.items():
        (VAULT / name).write_text(content, encoding='utf-8')


def parse_time(path):
    txt = pathlib.Path(path).read_text(errors='ignore') if pathlib.Path(path).exists() else ''
    out = {}
    for key in ['Elapsed (wall clock) time (h:mm:ss or m:ss)', 'Maximum resident set size (kbytes)', 'User time (seconds)', 'System time (seconds)']:
        m = re.search(r'\t' + re.escape(key) + r': (.*)', txt)
        if m:
            out[key] = m.group(1)
    return out


def run_json(cmd, name, timeout=300):
    out = ROOT / f'{name}.json'
    err = ROOT / f'{name}.stderr'
    tim = ROOT / f'{name}.time'
    full = ['/usr/bin/time', '-v', '-o', str(tim)] + [str(c) for c in cmd]
    t0 = time.time()
    p = subprocess.run(full, cwd=PROJECT, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
    elapsed_ms = int((time.time() - t0) * 1000)
    out.write_text(p.stdout, encoding='utf-8')
    err.write_text(p.stderr, encoding='utf-8')
    result = {'exit_code': p.returncode, 'elapsed_ms': elapsed_ms, 'time': parse_time(tim), 'stderr_tail': p.stderr[-1200:]}
    if p.returncode != 0:
        result['stdout_tail'] = p.stdout[-1200:]
        raise RuntimeError(f'{name} failed: {json.dumps(result, ensure_ascii=False)}')
    try:
        result['json'] = json.loads(p.stdout)
    except Exception as e:
        result['json_error'] = repr(e)
        result['stdout_tail'] = p.stdout[-1200:]
        raise
    return result


def api(method, path, body=None, timeout=300):
    data = None if body is None else json.dumps(body).encode('utf-8')
    req = urllib.request.Request(HS + path, data=data, method=method, headers={'Content-Type':'application/json'})
    t0 = time.time()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read().decode('utf-8')
            return {'status': r.status, 'elapsed_ms': int((time.time()-t0)*1000), 'json': json.loads(raw) if raw else None}
    except urllib.error.HTTPError as e:
        raw = e.read().decode('utf-8', errors='ignore')
        return {'status': e.code, 'elapsed_ms': int((time.time()-t0)*1000), 'error': raw[:2000]}


def hs_pids():
    out = subprocess.run("pgrep -f 'hindsight-api --host 127.0.0.1 --port 8765'", shell=True, text=True, stdout=subprocess.PIPE).stdout.split()
    return [int(x) for x in out if x.isdigit()]


def rss_kb(pid):
    try:
        txt = pathlib.Path(f'/proc/{pid}/status').read_text()
    except FileNotFoundError:
        return None
    m = re.search(r'^VmRSS:\s+(\d+)\s+kB', txt, re.M)
    return int(m.group(1)) if m else None


def hs_rss_snapshot():
    return {str(pid): rss_kb(pid) for pid in hs_pids()}


def rank_metrics(results, expected):
    expected_set = set(expected)
    ranks = []
    for i, path in enumerate(results, 1):
        if path in expected_set:
            ranks.append(i)
    return {
        'top1_hit': bool(ranks and ranks[0] == 1),
        'hit_at_3': bool(ranks and min(ranks) <= 3),
        'mrr': 0.0 if not ranks else round(1.0 / min(ranks), 4),
        'matched_ranks': ranks,
    }


def orderk_paths(resp):
    return [r['path'] for r in resp['results']]


def hs_paths(resp):
    paths = []
    for r in resp.get('results', []):
        doc = r.get('document_id') or ''
        if doc in DOCS:
            paths.append(doc)
        else:
            text = (r.get('text') or '').lower()
            matched = None
            for name, content in DOCS.items():
                title = content.split('\n', 5)[-1].split('\n', 1)[0].lower()
                if name.replace('.md','').lower() in text or title in text:
                    matched = name
                    break
            paths.append(matched or doc or '<unknown>')
    return paths


def main():
    clean()
    summary = {'root': str(ROOT), 'vault': str(VAULT), 'bank': BANK, 'docs': list(DOCS), 'queries': QUERIES}
    subprocess.run(['cargo','build','-p','orderk-cli','--all-features'], cwd=PROJECT, check=True)

    summary['sword_run'] = run_json([BIN, 'sword', 'run', '--vault', VAULT, '--thinking', 'active', '--max-files', '20', '--max-proposals', '12'], 'sword-active', timeout=360)
    summary['index'] = run_json([BIN, 'index', '--vault', VAULT, '--db', DB, '--embedding-provider', 'siliconflow', '--embedding-model', 'Qwen/Qwen3-Embedding-4B', '--embedding-dim', '1024'], 'orderk-index', timeout=180)

    orderk_rows = []
    for q in QUERIES:
        base = run_json([BIN, 'search', '--db', DB, '--query', q['query'], '--limit', '4', '--embedding-provider', 'siliconflow', '--embedding-model', 'Qwen/Qwen3-Embedding-4B', '--embedding-dim', '1024'], f"orderk-base-{q['id']}", timeout=120)
        sword = run_json([BIN, 'sword', 'search', '--vault', VAULT, '--db', DB, '--query', q['query'], '--limit', '4', '--embedding-provider', 'siliconflow', '--embedding-model', 'Qwen/Qwen3-Embedding-4B', '--embedding-dim', '1024'], f"orderk-sword-{q['id']}", timeout=120)
        base_paths = orderk_paths(base['json'])
        sword_paths = orderk_paths(sword['json'])
        orderk_rows.append({
            'id': q['id'], 'query': q['query'], 'expected': q['expected'],
            'base': {'paths': base_paths, 'metrics': rank_metrics(base_paths, q['expected']), 'elapsed_ms': base['elapsed_ms'], 'rss_kb': base['time'].get('Maximum resident set size (kbytes)'), 'took_ms': base['json'].get('took_ms')},
            'sword': {'paths': sword_paths, 'metrics': rank_metrics(sword_paths, q['expected']), 'elapsed_ms': sword['elapsed_ms'], 'rss_kb': sword['time'].get('Maximum resident set size (kbytes)'), 'took_ms': sword['json'].get('took_ms'), 'sidecar': sword['json'].get('sidecar')},
        })
    summary['orderk_eval'] = orderk_rows

    hs = {'rss_before': hs_rss_snapshot()}
    try:
        hs['create'] = api('PUT', f'/v1/default/banks/{BANK}', {
            'name': 'orderk sword isolated benchmark',
            'retain_mission': 'Extract factual claims from tiny benchmark Markdown documents. Preserve exact product/model names and document IDs. Ignore benchmark boilerplate.',
            'reflect_mission': 'Answer benchmark queries concisely from the retained Markdown facts only.',
            'enable_observations': False,
            'retain_extraction_mode': 'concise',
        })
        items=[]
        for doc_id, content in DOCS.items():
            items.append({'content': f'Document {doc_id}\n\n{content}', 'context': 'orderk sword benchmark markdown source', 'document_id': doc_id, 'tags': ['orderk-sword-bench'], 'timestamp': 'unset', 'metadata': {'source':'orderk-sword-bench','path':doc_id}})
        hs['retain'] = api('POST', f'/v1/default/banks/{BANK}/memories', {'items': items, 'async': False}, timeout=420)
        hs['rss_after_retain'] = hs_rss_snapshot()
        hs_rows=[]
        for q in QUERIES:
            before = hs_rss_snapshot()
            resp = api('POST', f'/v1/default/banks/{BANK}/memories/recall', {'query': q['query'], 'budget':'low', 'max_tokens':2048, 'trace': True, 'tags':['orderk-sword-bench'], 'tags_match':'all_strict'}, timeout=180)
            after = hs_rss_snapshot()
            paths = hs_paths(resp.get('json') or {})
            hs_rows.append({'id': q['id'], 'query': q['query'], 'expected': q['expected'], 'status': resp['status'], 'elapsed_ms': resp['elapsed_ms'], 'paths': paths, 'metrics': rank_metrics(paths, q['expected']), 'rss_before': before, 'rss_after': after, 'trace': (resp.get('json') or {}).get('trace')})
        hs['recall_eval'] = hs_rows
        hs['reflect_probe'] = api('POST', f'/v1/default/banks/{BANK}/reflect', {'query': '用三句话说明 Sword Spirit 应该从 Hindsight retrieval stack 借鉴什么，以及它为什么更轻量。', 'budget':'low', 'max_tokens':900, 'tags':['orderk-sword-bench'], 'tags_match':'all_strict'}, timeout=240)
    finally:
        hs['delete'] = api('DELETE', f'/v1/default/banks/{BANK}', None, timeout=120)
        hs['rss_after_delete'] = hs_rss_snapshot()
    summary['hindsight_eval'] = hs

    def aggregate(rows, key):
        vals=[r[key]['metrics'] for r in rows]
        return {
            'top1': sum(1 for m in vals if m['top1_hit']),
            'hit_at_3': sum(1 for m in vals if m['hit_at_3']),
            'mrr_avg': round(sum(m['mrr'] for m in vals)/len(vals), 4),
            'n': len(vals),
        }
    summary['aggregate'] = {
        'orderk_base': aggregate(orderk_rows, 'base'),
        'orderk_sword': aggregate(orderk_rows, 'sword'),
        'hindsight_recall': {
            'top1': sum(1 for r in hs.get('recall_eval', []) if r['metrics']['top1_hit']),
            'hit_at_3': sum(1 for r in hs.get('recall_eval', []) if r['metrics']['hit_at_3']),
            'mrr_avg': round(sum(r['metrics']['mrr'] for r in hs.get('recall_eval', []))/max(1,len(hs.get('recall_eval', []))), 4),
            'n': len(hs.get('recall_eval', [])),
        }
    }
    (ROOT / 'summary.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({
        'root': str(ROOT),
        'bank': BANK,
        'aggregate': summary['aggregate'],
        'sword_thinking': summary['sword_run']['json']['thinking'],
        'sword_resource': summary['sword_run']['time'],
        'index_resource': summary['index']['time'],
        'hs_retain_ms': hs.get('retain',{}).get('elapsed_ms'),
        'hs_reflect_ms': hs.get('reflect_probe',{}).get('elapsed_ms'),
        'hs_delete_status': hs.get('delete',{}).get('status'),
    }, ensure_ascii=False, indent=2))

if __name__ == '__main__':
    main()
