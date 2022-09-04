# What's this all about?

ns-rules allows you to enforce dependency rules between Clojure namespaces.

Consider the folowing imaginary Clojure codebase that handles shipping.

```
src/
  shipping/
    entity/
      ship.clj
      route.clj
      port.clj
      cargo_manifest.clj
      contract.clj
    service/
      database.clj
      event_log.clj
    use_case/
      cargo_assignment.clj
      routing.clj
      contract_verification.clj
    infrastructure/
      postgres.clj
      kafka.clj
```

This codebase has been written with certain assumptions about the dependencies
between the various namespaces. Specifically,

1. entities may only reference other entities,
1. services may only reference entities,
1. use_cases may only reference entities and services, and
1. infrastructure may refernce anything.

These rules are not enforced in anyway and over time, as teams change, such
rules are often forgotten to the detrement of the codebase.

ns-rules can enforce these rules with the following configuration.

ns-rules.edn
```edn
{:src-dirs ["src"]
 :rules    [shipping.entity.*   {:restrict-to [shipping.entity.*]}
            shipping.service.*  {:restrict-to [shipping.entity.*]}
            shipping.use_case.* {:restrict-to [shipping.entity.*
                                               shipping.service.*]}]}
``` 

If we run ns-rules in the project root we see that the rules are being obayed.

```bash
example $ ns-rules 
All checks passed
 12 files checked
  5 namespaces matched a rule
  0 warnings
  0 files skipped
```

However if we break a rule, ns-rules will tell us in great detail.

```bash
example $ ns-rules
────[namespace_rule_violation]────────────────────

    × 'shipping.entity.port' is not allowed to reference 'shipping.service.database'

   ╭───[src/shipping/entity/port.clj:1:1] shipping.entity.port:
 1 │ (ns shipping.entity.port
 2 │   (:require [shipping.service.database :as database]))
   ·              ────────────┬────────────
   ·                          ╰───────────── this reference is not allowed


Found 1 rule violation
 12 files checked
 10 namespaces matched a rule
  0 warnings
  0 files skipped
```

By calling ns-rules from a Git pre-commit hook you can ensure that the commit
will fail if your dependency rules are violated.