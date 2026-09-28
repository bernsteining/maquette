const hooks = {
  change() {},
  scrub() {},
  frame() { return Promise.resolve(); },
  recompile() {},
  viewChanged() {},
};

export { hooks };
