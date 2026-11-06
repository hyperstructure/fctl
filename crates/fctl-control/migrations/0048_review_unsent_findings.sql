-- Review findings that went to no session (fctl#395): one of the instance's
-- agents opened the pull request, but no session that lists it took them
-- when the review posted, since none listed it yet. The message its agent
-- would have got, as JSON (fctl_domain::reviews::Unsent), until a session
-- takes it: the session that opened the pull request once one lists it
-- (src/reviews.rs, `owed`), or the one a person passes it on to. NULL:
-- they went to a session, or there were none to pass on.
ALTER TABLE reviews ADD COLUMN unsent_findings TEXT;
