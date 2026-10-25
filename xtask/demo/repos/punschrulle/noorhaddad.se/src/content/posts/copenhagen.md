---
title: We're opening in Copenhagen
summary: Romanesco's second city, and what changes for the software.
date: 2026-09-14
---

Romanesco is opening in Copenhagen. Twelve farms on Zealand, one depot in
Valby, two vans to start with.

Most of the work isn't software. It's farms, drivers and a cold room. But
some of it is, and it's more than I expected.

The shop has only ever spoken Swedish and English. Now it needs Danish, and
prices in kroner. Every price in the code is an integer in öre, which helps:
we only have to teach the formatter a second currency.

Delivery days come from a depot. We've had one, so `sto-1` is hard-coded in
more places than I'd like to admit. Each of them will learn to ask which
depot it is.

The route planner is the hard part. Copenhagen's streets are new to it, and
it plans the vans' rounds in about 70 seconds. That's fine at night. It isn't
when a driver calls in sick at seven.

We'll write it all up once the first boxes are out.
