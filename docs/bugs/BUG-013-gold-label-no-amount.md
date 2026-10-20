# BUG-013: gold on the ground shows "Gold" without the amount

- **Reported:** 2026-10-16, test build 0.1.1.91 (owner).
- **What happens:** the label of a gold pile on the ground reads only "Gold".
- **Expected:** the label shows the amount, for example "1234 Gold".
- **Where to look:** the ground item label text for gold. The item's gold quantity probably isn't sent to the client, or isn't used when the label is built.
- **Status:** fixed on main (#141).
