Rebase Plan (Conventional Commits + Fixups)

Context
- Goal: produce a clear, conventional-commit history without mini-commits.
- Allowed: reword titles; fixup small/trivial commits into nearby relevant commits.
- Not allowed: manual reordering of feature-bearing commits (reduce conflict risk). Autosquash will move fixups next to their targets only.
- Command suggestion when ready: `git rebase -i --root --autosquash`

Legend
- Keep = keep commit as-is.
- Reword = change only the title to conventional form.
- Fixup -> <hash> = mark this commit as `fixup` into the target commit (low risk of conflicts, mostly docs/fmt-only crossings).

Plan (oldest → newest)

- bdd4e7b Initial commit
  - Reword → chore: initial commit

- de70d3d ci: add clippy+test and release-please (#1)
  - Keep

- 0372065 fix release-please ci
  - Reword → ci: fix release-please workflow

- 62b72e9 initial design doc
  - Reword → docs: initial design doc

- 6d72003 add analysis doc
  - Reword → docs: add analysis doc

- 99f2900 comments on analysis
  - Reword → docs: revise analysis comments

- 413022a design doc iteration
  - Reword → docs: design iteration

- b71c908 update design
  - Reword → docs: update design

- d85d972 design iteration
  - Reword → docs: design iteration

- e25dd4f remove analysis
  - Reword → docs: remove analysis

- 3ff6905 docs: split into three modules
  - Keep

- d497893 docs: switch to Rc-based keepalive
  - Keep

- 190035a add len and is_empty
  - Reword → docs: add len and is_empty

- 04726e2 add contains_key, iteration, clarify refcount rationale, move access to Ref, overflow semantics, reuse precomputed hash
  - Reword → docs: add contains_key/iteration spec; refine rationale

- cf64a21 fix iter_mut spec confusion
  - Reword → docs: clarify iter_mut spec

- 0d31887 renames, overflow UB standardization, OOM safety on insertion, fix iter_mut guard in wrong section, drop K,V before Inner to be defensive with unsafe K,V
  - Reword → docs: rename sections; standardize overflow/OOM semantics

- 79dd3a9 add accessor lifetime rationale
  - Reword → docs: add accessor lifetime rationale

- d98ccf4 manual edits / word-smithing
  - Reword → docs: copy edits

- ce35784 add Handle to Module1,2 and ManualRc for managing references
  - Reword → docs: document Handle in modules and ManualRc

- cb67907 feat: add ManualRc to increment/decrement Rc strong counts
  - Keep

- 9d48738 add Count and Token design
  - Reword → docs: add Count/Token design

- 0f5e49d initial move to Count/Token based
  - Reword → refactor: migrate to Count/Token model

- c025933 use handles of previous modules, remove redundant RcCount spec, standardize find() as method name for finding values by keys.
  - Reword → docs: standardize find(); remove redundant RcCount spec

- e4f646e specify token safety when insert fails
  - Reword → docs: specify token safety on failed insert

- da7a33e feat: add DebugReentrancy to check re-entrancy contraints
  - Keep

- 63e58e8 add reentrancy guard, fix reference safety in iterator
  - Reword → docs: add reentrancy guard rationale; iterator safety

- 32fb40d feat: add RcHashMap
  - Keep

- fca2433 add tests to HandleHashMap
  - Reword → test: add HandleHashMap tests

- 7bbb469 run cargo fmt
  - Fixup → 46a4436 (format-only; immediately follows benches)

- 46a4436 add benchmarks
  - Reword → chore(bench): add benchmarks

- 3ad48bb add prop testing for handle map
  - Reword → test: add HandleHashMap property tests

- 00b5180 remove "raw" functions in UsizeCount
  - Reword → refactor: remove UsizeCount raw functions

- 697160d speed up property tests for HandleHashMap
  - Reword → test: speed up HandleHashMap property tests

- 4e1eb0d some fixes to CountedHashMap and RcHashMap
  - Reword → fix: CountedHashMap and RcHashMap issues

- b3178cf add accessors and iterators to CountedHashMap
  - Reword → feat: add accessors and iterators to CountedHashMap

- 9ab29f1 some edits to RcHashMap
  - Reword → refactor: tidy RcHashMap internals

- f3aa7ea return static lifetime tokens and hold Token in CountedHandle
  - Reword → refactor: return 'static tokens; store Token in CountedHandle

- 133ac9a put panics on stale handles - shouldn't happen
  - Reword → fix: panic on stale handles in put

- 53b9de2 optimize insert for happy path; cargo fmt
  - Reword → perf: optimize insert happy path

- 5ef2425 add find<Q> to module 1,2 and note CountedHashMap::put receives &mut self
  - Reword → feat: add find<Q>(); make CountedHashMap::put take &mut self

- 1afaa73 add token branding,  fix rc_map
  - Reword → refactor: add token branding; fix rc_map

- a50a25a simplify variable names in counted_map
  - Reword → style: simplify variable names in CountedHashMap

- ccbe92f add branding ADT
  - Reword → docs: add branding ADT

- 27d1098 simplify iteration in CountedHashMap
  - Reword → refactor: simplify CountedHashMap iteration

- af70154 simplify token impl
  - Reword → refactor: simplify token implementation

- ec282fd rename files
  - Reword → refactor: rename files for clarity

- de1622d update token docs
  - Reword → docs: update token docs

- 3b2148b save RcCount Token in RcVal
  - Reword → refactor: store RcCount Token in RcVal

- c458f73 chore(deps) update hashbrown to 0.16
  - Keep

- 04e2ccd test: add token tests
  - Keep

- 3592b07 test: add CountedHashMap proptest
  - Keep

- 1f4d8fd chore: run cargo fmt
  - Fixup → 8816c7c (format-only; crosses a docs-only commit, low risk)

- b742564 docs: update tokens.md
  - Keep

- 8816c7c feat: add insert_with() in lower_level, simplifies RcHashMap::insert()
  - Keep

- b769887 docs: update CountedHandle returning 'static lifetime
  - Keep

- 2549a92 feat: reduce unsafe by returning 'static lifetime CountedHandle from CountedHashMap
  - Optional reword → feat: return 'static CountedHandle; reduce unsafe


Execution Notes
- Use `reword` for the commits above to edit titles; keep bodies as-is.
- Mark the listed `fixup` commits as `fixup` in the interactive todo list, then run with `--autosquash`.
- Because fixups only cross docs-only commits (or are adjacent), conflict risk is minimal.
- After rebase, verify `release-please` still recognizes types (feat, fix, docs, chore, refactor, test, perf, ci, style).

Verification Checklist
- `git log --oneline` shows only conventional-commit subjects.
- No stray fmt-only commits remain; they are squashed.
- Docs-only changes remain separate where meaningful.
- CI/release configuration commits retain `ci:` prefix.

