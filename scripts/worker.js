// Static-asset Worker: the emulator is entirely client side, so all requests
// are served straight from the built `web/` directory.
export default {
  fetch(request, env) {
    return env.ASSETS.fetch(request);
  },
};
