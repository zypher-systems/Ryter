# ledger

Amounts in several currencies.

    sh dev build   # data/rates.csv -> build/rates.json
    sh dev lint
    sh dev test    # builds, then runs the tests

The tests need `build/rates.json` and `src` on the path, so run them through
`dev`.
