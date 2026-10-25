# Agents on the night shift

Noor Haddad, Romanesco

---

## Romanesco

- Vegetables, bread and eggs from 40 farms around Stockholm
- About 9,000 subscribers
- Eight of us write software

---

## The evening problem

- A migration
- A new route planner
- A framework upgrade

Not hard. Long.

---

## A night

```
22:10  ●  I write the task and start a session
22:55  ●  The agent opens a pull request
22:57  ●  The review agent: 2 issues
23:20  ●  The agent fixes both and pushes
23:23  ●  The review agent: no issues
          ⋮   (I sleep)
07:50  ●  I read it over breakfast
08:05  ●  Merged
```

---

## What goes in the evening

- ✓ "Cache the slot query and show me the numbers before and after"
- ✗ "Make the checkout nicer"

If I can't say how I'd check it, it doesn't start at night.

---

## A second pair of eyes

- Every pull request, reviewed by another agent
- It doesn't approve anything
- It finds what a tired person misses

---

## The loop

```
 You ──task──▶ Agent ──pull request──▶ Review agent
                 ▲                          │
                 └───findings, until none───┘
 You ◀──────── no issues, checks pass ─────┘
      read it, merge
```

---

## Notes for the next agent

- "Never log a stop: it carries the door code."
- "Run `cargo sqlx prepare` after you change a query, or CI fails."

---

## The machines

- Two Macs in the office
- Two rented Linux servers
- My laptop

---

## Limits

- Plans run out; sessions wait
- Long nightly work on the bigger plans
- Not everything at 02:00

---

## What's still hard

- Vague tasks
- Anything visual
- Flaky tests

---

## Thank you

The agents work nights. We still read everything in the morning.
