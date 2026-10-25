# Driver app

The app Romanesco's drivers deliver with: the day's route, door codes at the door, and a photo on the doorstep when nobody answers. Expo and React Native, in TypeScript.

## Running it

```sh
npm ci
npx expo start                     # then i for the iOS simulator, or scan the code with a phone
npm test                           # Jest, with jest-expo
npm run lint                       # ESLint, with rules of our own in eslint-rules/
npm run typecheck
npx expo export --platform ios     # check that the app bundles
```

The route comes from the platform's drivers API at `EXPO_PUBLIC_API_URL` (staging's when it isn't set). Sign in with a driver's one-time code from staging's admin.

## Builds

The native projects aren't in the repository: `npx expo prebuild` makes them from `app.json` and its config plugins. Builds run on EAS from the `eas-build` workflow: every push to `main` makes an internal build for the drivers' test phones, and every tag one for the App Store. The maps need `MAPBOX_DOWNLOADS_TOKEN` at build time.

## Layout

- `app/`: the screens, with Expo Router.
- `src/route/`: the day's route and its stops.
- `src/delivery/`: what happens at a stop, and the outbox that keeps it on the phone until it's online.
- `src/map/`: the map, and the route area's offline tiles.
- `src/api/`: the platform's drivers API.
- `eslint-rules/`: lint rules of our own.
