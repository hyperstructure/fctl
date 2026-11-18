// A view transition the page starts while it is hidden is skipped, and its `ready` promise rejects.
// The router starts one for every page it opens, the first one too, and iOS starts the app on an
// iPhone's Home Screen in the background at times. The specification marks that promise handled,
// but WebKit leaves the rejection for nothing to catch, and the app log kept it as an error
// ("View transition was skipped because document visibility state is hidden").

/** Has every view transition's `ready` promise caught, as the specification has a skipped one's. */
export function catchSkippedTransitions() {
  const start = document.startViewTransition?.bind(document);
  if (!start) return;
  document.startViewTransition = (update) => {
    const transition = start(update);
    transition.ready.catch(() => {});
    return transition;
  };
}
