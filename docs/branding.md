Branding Tokens and Counters

Decision
- We evaluated per-instance branding (generative `'id` lifetimes) to make tokens returnable only to the exact counter that minted them at compile time. While feasible, it requires a higher‑ranked closure (GhostCell/QCell pattern) or explicit brand parameters threaded through APIs. This hurts ergonomics for our use case.
- We are not adopting branding in the public or internal APIs right now.

Why borrow-branding (via `&self`) isn’t enough
- Tying `Token<'a, C>` to `&'a self` makes the token’s lifetime equal to a borrow, which can be the same for two different instances borrowed in the same scope. The compiler can pick one `'a` that satisfies both borrows, so a token from `a` can accidentally type-check with `b.put`.
- To make two instances incompatible, you need an additional type-level identity — a generative brand that differs across instances, not just across borrows.

Current approach
- Tokens remain zero-sized and linear; dropping a token still panics to catch misuse.

Why we’re comfortable with this
- Tokens are an internal flow-control mechanism; APIs are structured so callers can only obtain tokens through the map and must return them via the map.
- The linear, non-Clone token plus panic-on-Drop catches misuse in testing without complicating the API surface.
