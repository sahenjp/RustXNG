#!/usr/bin/env python3
"""Choose publication or PR preparation without replaying an unpublished version."""
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

from release_tools import version


def plan(current, manifest, releases, tag_exists, created=False, tag="", sha=""):
    wanted = 'v' + version('v' + current)
    if manifest != current:
        raise ValueError('package and release manifest versions differ')
    if created:
        if tag != wanted or not re.fullmatch(r'[0-9a-f]{40}', sha):
            raise ValueError('new draft output does not identify the current version and commit')
    else:
        if tag or sha:
            raise ValueError('unexpected release outputs without a created release')
        pending = [r for r in releases if r.get('draft') and r.get('tag_name') == wanted
                   and (r.get('author') or {}).get('login') == 'github-actions[bot]']
        if len(pending) > 1:
            raise ValueError('ambiguous pending release drafts')
        if pending:
            tag, sha = wanted, pending[0].get('target_commitish', '')
            if not re.fullmatch(r'[0-9a-f]{40}', sha):
                raise ValueError('draft target must be an exact source commit')
        else:
            published = [r for r in releases if r.get('tag_name') == wanted
                         and not r.get('draft') and not r.get('prerelease')]
            if len(published) != 1 or not tag_exists:
                raise ValueError('current version has no published release/tag; refuse to replay history')
    publishing = bool(tag and sha)
    return {'created': str(publishing).lower(), 'prepare_pr': str(not publishing).lower(),
            'tag': tag, 'sha': sha}


def main():
    repo = os.environ['GITHUB_REPOSITORY']

    def api(path):
        return json.loads(subprocess.check_output(['gh', 'api', f'repos/{repo}/{path}']))

    current = tomllib.loads(Path('Cargo.toml').read_text(encoding='utf-8'))['package']['version']
    manifest = json.loads(Path('.release-please-manifest.json').read_text(encoding='utf-8'))['.']
    wanted = 'v' + version('v' + current)
    releases = api('releases?per_page=100')
    flag = os.environ.get('CREATED', '')
    if flag not in ('', 'false', 'true'):
        raise ValueError('invalid release-created output')
    tag, sha = os.environ.get('TAG', ''), os.environ.get('SOURCE_SHA', '')
    pending = [r for r in releases if r.get('draft') and r.get('tag_name') == wanted
               and (r.get('author') or {}).get('login') == 'github-actions[bot]']
    for release in pending:
        release['target_commitish'] = api('commits/' + release['target_commitish'])['sha']
    tag_exists = False
    if flag != 'true' and not pending:
        # Missing tags or API errors fail the job, never bootstrap the entire history.
        api('git/ref/tags/' + wanted)
        tag_exists = True
    result = plan(current, manifest, releases, tag_exists, flag == 'true', tag, sha)
    with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as stream:
        for key, value in result.items():
            stream.write(f'{key}={value}\n')
    print('Release state:', 'publish existing/current draft' if result['created'] == 'true'
          else 'published baseline verified; PR preparation allowed')


if __name__ == '__main__':
    main()
