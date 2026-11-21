# Monorepo fixture

Three packages (`alpha`, `beta`, `app`) with a linear dependency chain. The
workspace defaults select `alpha` and `beta`; `app` is selected with `-p app` or
`--workspace`.

Useful commands:

```sh
seal graph packages --project ztests/projects/monorepo
seal graph modules --project ztests/projects/monorepo --format mermaid
seal affected --project ztests/projects/monorepo --since HEAD
seal doc --project ztests/projects/monorepo -p alpha
seal api --project ztests/projects/monorepo -p alpha --json
seal task ci --project ztests/projects/monorepo
```
