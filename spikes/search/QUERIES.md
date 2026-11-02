# S3 pre-registered queries

Fixed on 2026-11-02, **before** testing any source (design.md S3, external review R2.2).
Do not edit these after the first test run; if a query must change, add a dated note explaining why.

Pass rule: a source passes if, for at least 7 of 10 queries, the owner judges that the short spoken
summary actually answers the question. Record search-path latency separately.

| # | Lang | Type | Query |
|---|---|---|---|
| 1 | es | stable fact | ¿Cuál es la capital de Australia? |
| 2 | en | stable fact | How tall is Mount Everest? |
| 3 | es | stable fact | ¿Quién escribió Cien años de soledad? |
| 4 | es | time-sensitive | ¿Quién ganó la última Champions League? |
| 5 | en | time-sensitive | What is the price of Bitcoin today? |
| 6 | es | time-sensitive | ¿Cómo está el clima hoy en Buenos Aires? |
| 7 | en | time-sensitive | What is the latest news about the Windows 11 update? |
| 8 | en | likely answerable | What is photosynthesis? |
| 9 | es | likely answerable | ¿Qué es un agujero negro? |
| 10 | en | likely answerable | Who painted the Mona Lisa? |

## Results

| # | Source | Answered? (owner) | Latency | Notes |
|---|---|---|---|---|
