# Agents on the night shift: speaker notes

## 1. Title

Hi, I'm Noor. I run the software at Romanesco. This is about what we let
coding agents do while we sleep, and what we don't.

## 2. Romanesco

We deliver boxes of vegetables, bread and eggs from 40 farms around
Stockholm. About 9,000 subscribers. Eight of us write the software: the
shop, the platform behind it, the app our drivers use. We're small. There's
always more work than evenings.

## 3. The evening problem

Some work needs a quiet evening. A migration. Moving route planning to a
new solver. A framework upgrade. None of it is hard. It's long: two hours of
reading, two hours of careful typing, then waiting for CI. For a year that
work happened when someone gave up an evening. Usually me. (Pause. Someone
always laughs here.)

## 4. A night

So now an agent does the long part. This is a real night from last month.
At 22:10 I write the task and start a session. By 22:55 there's a pull
request. Two minutes later a second agent has reviewed it and found two
problems. The first one fixes both. At 23:23 the review is clean and I go to
bed. At 07:50 I read it with my coffee. At 08:05 it's merged. That's the
whole trick, really. Everything else is about making that night go well.

## 5. What goes in the evening

Not everything works. A task with a clear end works. "Cache the slot query
and show me the numbers before and after": that works. "Make the checkout
nicer" doesn't. My rule: if I can't say how I'd check it, I don't start it
at night. Raise your hand if you've written a ticket that says "make it
nicer". Yes. Me too.

## 6. A second pair of eyes

Every pull request gets reviewed by another agent. It reads the diff fresh.
It doesn't approve anything, and it isn't a person. But it finds the things
a tired person misses. A migration that would have locked the orders table
for four minutes. A time zone that was an hour off, but only in winter.

## 7. The loop

Here's the loop. You write the task. The agent works and opens a pull
request. The review agent reads it, and its findings go back to the agent
until there are none. When the checks pass, a person reads it and merges.
The person is the last step. That isn't going to change.

## 8. Notes for the next agent

Agents start from nothing every time. So they leave notes for the next one.
"Never log a stop: it carries the door code." "Run cargo sqlx prepare after
you change a query, or CI fails." After a few months that's the best
onboarding document we have. New people read it too. Our newest engineer
read all of it in her first week. (Check with Sanna that it's fine to say.)

## 9. The machines

People expect a data centre. It's two Macs in the office, two rented Linux
servers and my laptop. fctl runs the sessions on them, each in its own
sandbox. When a machine needs a kernel update we drain it. The sessions on
it finish, and nobody notices.

## 10. Limits

Plans have limits. A Pro plan can run out halfway through a long session,
and then the session waits until it resets. So the long nightly work runs on
the bigger plans, and we don't start everything at 02:00.

## 11. What's still hard

Vague tasks, see above. Anything visual: an agent can take a screenshot,
but it has no taste. And flaky tests: an agent will happily retry a flaky
test until it passes. So we told it not to.

## 12. Thank you

The agents work nights. We still read everything in the morning. Thank you.
