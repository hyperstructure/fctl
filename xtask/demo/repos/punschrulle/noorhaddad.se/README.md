# noorhaddad.se

My site: notes on building Romanesco, recipes and the odd talk. Astro, on
Cloudflare Pages.

```sh
npm install
npm run dev     # http://localhost:4321, drafts included
npm run build   # to dist/, without drafts
```

Posts are Markdown in `src/content/posts/`. A post stays a draft
(`draft: true`) until it has a date. Talks keep their slides and speaker
notes in `talks/`, which the site doesn't publish.
