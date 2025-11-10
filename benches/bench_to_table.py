#!/usr/bin/env python3
import argparse
import json
import math
from collections import defaultdict, OrderedDict


DS_NAME_MAP = {
    'handle': 'HandleHashMap',
    'counted': 'CountedHashMap',
    'rc': 'RcHashMap',
}

COL_ORDER = ['HandleHashMap', 'CountedHashMap', 'RcHashMap']


def time_to_seconds(value, unit):
    # Normalize typical time to seconds
    unit = (unit or '').lower()
    if unit == 's':
        return value
    if unit == 'ms':
        return value / 1e3
    if unit == 'us' or unit == 'µs':
        return value / 1e6
    # default to ns
    return value / 1e9


def case_label(category, case):
    # Map known cases to friendly labels; fall back to suffix
    mapping = {
        ('insert', 'fresh_100k'): 'insert fresh',
        ('insert', 'warm_100k'): 'insert warm',
        ('remove', 'random_10k_of_110k'): 'remove random',
        ('query', 'hit_10k_on_100k'): 'query hit',
        ('query', 'miss_10k_on_100k'): 'query miss',
        ('access', 'random_increment_100k'): 'access random increment',
        ('access', 'iter_all_100k'): 'access iter all',
        ('access', 'iter_mut_increment_all_100k'): 'access iter_mut increment all',
    }
    return mapping.get((category, case), f"{category}/{case}")


def parse_id(bench_id):
    # Expect format: <ds>::<category>/<case>
    try:
        ds_and_cat, case = bench_id.split('/')
        ds_prefix, category = ds_and_cat.split('::')
    except ValueError:
        return None
    return ds_prefix, category, case


def load_results(path):
    rows = defaultdict(dict)  # row_label -> { ds_name -> value }
    with open(path, 'r') as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue

            if msg.get('reason') != 'benchmark-complete':
                continue

            bench_id = msg.get('id')
            parsed = parse_id(bench_id) if bench_id else None
            if not parsed:
                continue
            ds_prefix, category, case = parsed
            ds_name = DS_NAME_MAP.get(ds_prefix)
            if not ds_name:
                continue

            typical = msg.get('typical') or {}
            typical_est = typical.get('estimate')
            typical_unit = typical.get('unit', 'ns')
            if typical_est is None:
                # fall back to mean if typical missing
                mean = msg.get('mean') or {}
                typical_est = mean.get('estimate')
                typical_unit = mean.get('unit', 'ns')
            if typical_est is None:
                continue

            # find element-based throughput
            per_iter = None
            for th in msg.get('throughput') or []:
                if th.get('unit') == 'elements':
                    per_iter = th.get('per_iteration')
                    break
            if per_iter is None:
                continue

            typical_s = time_to_seconds(typical_est, typical_unit)
            if typical_s <= 0:
                continue

            elems_per_sec = per_iter / typical_s
            m_elems_per_sec = elems_per_sec / 1e6

            label = case_label(category, case)
            rows[label][ds_name] = m_elems_per_sec

    return rows


def format_table(rows):
    # Header
    header = ['Test Case'] + COL_ORDER
    out = []
    out.append('| ' + ' | '.join(header) + ' |')
    out.append('| ' + ' | '.join(['---'] * len(header)) + ' |')

    # Stable row order: sorted by our known categories, then alpha
    def row_sort_key(name):
        priority = [
            'insert fresh', 'insert warm',
            'remove random',
            'query hit', 'query miss',
            'access random increment', 'access iter all', 'access iter_mut increment all',
        ]
        try:
            return (0, priority.index(name))
        except ValueError:
            return (1, name)

    for row_name in sorted(rows.keys(), key=row_sort_key):
        values = rows[row_name]
        cells = [row_name]
        for col in COL_ORDER:
            v = values.get(col)
            cells.append(f"{v:.2f}" if isinstance(v, (int, float)) else '-')
        out.append('| ' + ' | '.join(cells) + ' |')
    return '\n'.join(out)


def main():
    ap = argparse.ArgumentParser(description='Convert cargo-criterion JSONL to Markdown throughput table (M elems/sec).')
    ap.add_argument('jsonl', help='Path to JSONL produced by cargo criterion')
    args = ap.parse_args()

    rows = load_results(args.jsonl)
    table = format_table(rows)
    print(table)


if __name__ == '__main__':
    main()

