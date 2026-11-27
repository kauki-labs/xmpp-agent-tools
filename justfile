# Unit tests: standard library only, no network.
test:
    PYTHONPATH=src python3 -m unittest discover -s tests -t . -v

# Branch coverage with a floor of 90% (needs `pip install coverage`).
coverage:
    PYTHONPATH=src python3 -m coverage run --branch --source=src -m unittest discover -s tests -t .
    python3 -m coverage report -m --fail-under=90

# Every one-line mutation must fail the tests.
mutate:
    python3 scripts/mutation-check.py

# No internal names, addresses or home paths in this public repository.
scrub:
    python3 ci/scrub.py

check: test mutate scrub
