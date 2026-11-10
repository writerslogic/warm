# Copyright 2024-2026 WritersLogic Contributors
# SPDX-License-Identifier: Apache-2.0
"""Verify complete flagged-review provenance without accepting semantic proposals."""
import argparse
import copy
from collections import Counter
import hashlib
import json
from pathlib import Path
import re


def require(condition, message):
    if not condition:
        raise ValueError(message)


def resolve(document, pointer):
    require(pointer.startswith('/'), 'absolute JSON pointer required')
    for part in pointer[1:].split('/'):
        key = part.replace('~1', '/').replace('~0', '~')
        document = document[int(key)] if isinstance(document, list) else document[key]
    return document


def verify(index, directory, overlay_path):
    def pinned(path, expected):
        raw = path.read_bytes()
        require(hashlib.sha256(raw).hexdigest() == expected, 'hash mismatch: ' + path.name)
        return json.loads(raw)

    def local(relative):
        path = (directory / relative).resolve()
        require(directory.resolve() in path.parents, 'review path escapes artifact directory')
        return path

    source = pinned(local('sources/locomo10.json'), index['source_sha256'])
    external, reports, reviewed = {}, {}, {}
    for item in index['external_claim_files']:
        claims = pinned(local(item['path']), item['sha256'])
        require(len(claims) == item['claim_count'], 'external count mismatch')
        external[item['path']] = claims
    external_ids = [row['question_id'] for rows in external.values() for row in rows]
    require(len(set(external_ids)) == len(external_ids), 'duplicate external identity')
    for item in index['review_reports']:
        report = pinned(local(item['path']), item['sha256'])
        reports[item['path']] = report
        require(len(report['reviewed']) == item['review_count'], 'review count mismatch')
        for review in report['reviewed']:
            require(review['id'] not in reviewed, 'duplicate reviewed identity')
            require(review.get('source_sha256', report.get('source_sha256')) == index['source_sha256'], 'review source mismatch')
            reviewed[review['id']] = review
    overlay = pinned(overlay_path, index['root_acceptance_artifact']['sha256'])
    require(overlay['dataset_sha256'] == index['source_sha256'], 'overlay source mismatch')
    accepted = {row['id']: row for row in overlay['accepted_source_corrections']}
    seen, seen_acceptances, references = set(), set(), 0
    for entry in index['entries']:
        qid = entry['id']
        match = re.fullmatch(r'locomo_(\d+)_qa(\d+)', qid)
        require(match is not None and qid not in seen, 'duplicate or invalid indexed identity')
        seen.add(qid)
        conversation, question = map(int, match.groups())
        pointer = f'/{conversation}/qa/{question}'
        require(entry['source_question_pointer'] == pointer, 'identity pointer mismatch')
        official = resolve(source, pointer)
        review = resolve(reports[entry['review_file']], entry['review_pointer'])
        require(review['id'] == qid and review['official'] == official, 'review question mismatch')
        require(entry['review_decision'] == review['decision'], 'decision mismatch')
        require(entry['source_category'] == official['category'], 'category mismatch')
        file, fragment = entry['external_claim_pointer'].split('#')
        claim = resolve(external[file], fragment)
        require(claim['question_id'] == qid and claim['error_type'] == entry['external_error_type'], 'external linkage mismatch')
        if 'external_claim' in review:
            require(review['external_claim'] == claim, 'embedded external claim mismatch')
        if 'official_pointer' in review:
            require(review['official_pointer'] == pointer, 'review identity mismatch')
        for identity, evidence in review['evidence'].items():
            if 'pointer' in evidence:
                turn = resolve(source, evidence['pointer'])
                require(evidence['pointer'].startswith(f'/{conversation}/conversation/'), 'evidence conversation mismatch')
                require(resolve(source, evidence['session_date_pointer']) == evidence['session_date'], 'date mismatch')
            else:
                turns = [turn for key, session in source[conversation]['conversation'].items() if re.fullmatch(r'session_\d+', key) for turn in session if turn['dia_id'] == identity]
                require(len(turns) == 1, 'legacy evidence identity is not unique')
                turn = turns[0]
            require(turn['dia_id'] == identity and all(turn[key] == evidence[key] for key in ('speaker', 'text')), 'evidence content mismatch')
            references += 1
        root = entry['root_source_review']
        if root['status'] == 'independently_accepted_source_correction':
            require(qid in accepted and resolve(overlay, root['pointer']) == accepted[qid] and root['answer'] == accepted[qid]['answer'], 'acceptance overlay mismatch')
            seen_acceptances.add(qid)
        elif root['status'] == 'other_independent_E7_decision':
            require(resolve(overlay, root['pointer']) == overlay['other_decisions'][qid] == root['decision'], 'other overlay mismatch')
        else:
            require(root['status'] == 'no_accepted_source_correction_recorded_in_E7' and qid not in accepted, 'unsupported acceptance status')
        require(entry['applied_to_official_key'] is False and entry['applied_to_system'] is False and entry['scored'] is False and entry['score_gain_claim'] is None, 'unexpected application or score claim')
    require(seen == set(external_ids) == set(reviewed), 'incomplete flagged coverage')
    require(seen_acceptances == set(accepted), 'incomplete acceptance overlay')
    coverage = index['coverage']
    require(all(coverage[key] == len(seen) for key in ('external_claim_count', 'indexed_claim_count', 'unique_indexed_ids')), 'index count mismatch')
    require(dict(Counter(e['external_error_type'] for e in index['entries'])) == coverage['error_type_counts'], 'type count mismatch')
    require(dict(Counter(e['review_disposition'] for e in index['entries'])) == coverage['review_disposition_counts'], 'disposition count mismatch')
    require(coverage['root_accepted_source_correction_count'] == len(accepted), 'acceptance count mismatch')
    return {'claims': len(seen), 'source_references': references, 'existing_root_corrections': len(accepted), 'new_semantic_acceptances': 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--index', required=True, type=Path)
    parser.add_argument('--overlay', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    require(not args.output.exists(), 'output exists')
    index = json.loads(args.index.read_text())
    result = verify(index, args.index.parent, args.overlay)
    rejected = []
    for name in ('duplicate', 'missing', 'wrong_pointer'):
        bad = copy.deepcopy(index)
        if name == 'duplicate':
            bad['entries'].append(copy.deepcopy(bad['entries'][0]))
        elif name == 'missing':
            bad['entries'].pop()
        else:
            bad['entries'][0]['source_question_pointer'] = '/0/qa/999999'
        try:
            verify(bad, args.index.parent, args.overlay)
        except ValueError:
            rejected.append(name)
        else:
            raise AssertionError('malformed index accepted: ' + name)
    result.update({'decision': 'accept-structural-only', 'rejected_mutations': rejected, 'index_sha256': hashlib.sha256(args.index.read_bytes()).hexdigest(), 'whole_key_audited': False, 'official_score': None, 'audited_score': None})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open('x') as handle:
        json.dump(result, handle, indent=2)
        handle.write('\n')
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    main()
