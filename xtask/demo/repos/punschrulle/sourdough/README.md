# sourdough

Feeding reminders for my starter, and a bake-day plan that works back from
when the bread should be ready.

A SvelteKit app with a small SQLite database. It runs on the mini PC under
the TV (`fika`) and lives on my phone's home screen.

```sh
npm install
npm run dev        # http://localhost:5173
npm test           # Vitest
npm run check      # svelte-check
```

To deploy: `npm run build`, then `node build` on fika (adapter-node), with
`DATABASE_PATH` pointing at the database and `TZ=Europe/Stockholm`.
