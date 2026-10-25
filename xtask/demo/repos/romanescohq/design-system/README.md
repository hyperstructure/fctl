# design-system

Romanesco's design tokens and React components, which the shop and the driver app share. Lucía looks after it: changes to the tokens need her review.

- `tokens.json`: colors, type, spacing and radii, in the W3C design tokens format.
- `src/`: the shop's components, each with a story.

```sh
npm run build       # tokens.json → dist/tokens.css and dist/tokens.ts
npm run storybook   # the components, at localhost:6006
```

The shop imports `@romanescohq/design-system`. The driver app copies the colors it uses into its `src/theme.ts`.
