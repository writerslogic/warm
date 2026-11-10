# Documentation API tools

Run `npm ci` and `npm test` in this directory before generating the Node API reference.

The minimatch override includes the fix for GHSA-g3hv-75wg-m249. TypeDoc 0.28.20
uses minimatch's removed private `matchOne` method for directory traversal, so
the install script replaces that call with the public partial-match API. The
patch accepts only the reviewed TypeDoc version and source hash and fails if
either changes. Reassess it when upgrading TypeDoc; retain the traversal and
bounded-expansion regression tests.
