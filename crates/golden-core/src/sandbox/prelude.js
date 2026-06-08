// golden-core JS prelude. Provides the pm.test runtime and a Chai-BDD subset for
// pm.expect. Native bindings (pm.response/request/environment/... and __pm_record)
// are injected from Rust BEFORE this prelude is evaluated.

(function () {
  // ---- atob / btoa (base64 over latin1) — quickjs ships neither ------------
  (function () {
    var T = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    if (typeof globalThis.btoa === 'undefined') {
      globalThis.btoa = function (s) {
        s = String(s);
        var out = '', i = 0;
        for (; i + 2 < s.length; i += 3) {
          var n = (s.charCodeAt(i) << 16) | (s.charCodeAt(i + 1) << 8) | s.charCodeAt(i + 2);
          out += T[(n >> 18) & 63] + T[(n >> 12) & 63] + T[(n >> 6) & 63] + T[n & 63];
        }
        var rem = s.length - i;
        if (rem === 1) {
          var a = s.charCodeAt(i) << 16;
          out += T[(a >> 18) & 63] + T[(a >> 12) & 63] + '==';
        } else if (rem === 2) {
          var b = (s.charCodeAt(i) << 16) | (s.charCodeAt(i + 1) << 8);
          out += T[(b >> 18) & 63] + T[(b >> 12) & 63] + T[(b >> 6) & 63] + '=';
        }
        return out;
      };
    }
    if (typeof globalThis.atob === 'undefined') {
      globalThis.atob = function (b) {
        b = String(b).replace(/=+$/, '');
        var out = '', bits = 0, val = 0;
        for (var i = 0; i < b.length; i++) {
          var idx = T.indexOf(b.charAt(i));
          if (idx === -1) continue;
          val = (val << 6) | idx; bits += 6;
          if (bits >= 8) { bits -= 8; out += String.fromCharCode((val >> bits) & 0xff); }
        }
        return out;
      };
    }
  })();

  // ---- pm.test -------------------------------------------------------------
  // __pm_record(name, passed, error|null) is a native fn that pushes into the
  // Rust-side collector.
  pm.test = function (name, fn) {
    try {
      fn();
      __pm_record(String(name), true, null);
    } catch (e) {
      var msg = (e && e.message != null) ? String(e.message) : String(e);
      __pm_record(String(name), false, msg);
    }
  };

  // ---- pm.expect backed by bundled chai 4.4.1 (spike-validated under rquickjs).
  // chai.js is prepended to the prelude at build time via include_str! concat.
  pm.expect = chai.expect;

  // ---- Register Postman-style .to.have.status(n) chai plugin ----------------
  // Stock chai 4.4.1 has no .status assertion; Postman adds it via chai.use().
  // The subject of pm.response.to is pm.response (has a .code property).
  chai.use(function (ch, utils) {
    ch.Assertion.addMethod('status', function (expected) {
      var obj = this._obj;
      var actual = (obj && typeof obj === 'object' && 'code' in obj) ? obj.code : obj;
      this.assert(
        actual === expected,
        'expected response to have status #{exp} but got #{act}',
        'expected response not to have status #{exp}',
        expected,
        actual
      );
    });
  });

  // ---- pm.request ----------------------------------------------------------
  (function () {
    var rq = (typeof __pm_request !== "undefined") ? __pm_request : {};
    var headers = {
      get: function (k) { return rq.__headers ? rq.__headers[String(k).toLowerCase()] : undefined; },
    };
    // case-insensitive header lookup
    headers.get = function (k) {
      if (!rq.__headers) return undefined;
      var want = String(k).toLowerCase();
      for (var key in rq.__headers) {
        if (key.toLowerCase() === want) return rq.__headers[key];
      }
      return undefined;
    };
    pm.request = {
      method: rq.method,
      url: rq.url,
      body: rq.body,
      headers: headers,
    };
  })();

  // ---- pm.response ---------------------------------------------------------
  if (typeof __pm_response !== "undefined") {
    var rs = __pm_response;
    var rheaders = {
      get: function (k) {
        var want = String(k).toLowerCase();
        for (var key in rs.__headers) {
          if (key.toLowerCase() === want) return rs.__headers[key];
        }
        return undefined;
      },
    };
    pm.response = {
      code: rs.code,
      status: rs.status,
      responseTime: rs.responseTime,
      headers: rheaders,
      text: function () { return rs.text; },
      json: function () { return JSON.parse(rs.text); },
    };
    // pm.response.to — getter returns a fresh chai assertion on pm.response,
    // enabling pm.response.to.have.status(n) via the registered chai plugin.
    Object.defineProperty(pm.response, 'to', {
      get: function () { return pm.expect(pm.response); },
    });
  }

  // ---- pm.<scope> : environment / globals / collectionVariables / variables -
  function makeScope(scopeName) {
    return {
      get: function (k) {
        var v = (typeof __pm_vars !== "undefined") ? __pm_vars[k] : undefined;
        return v;
      },
      has: function (k) {
        return (typeof __pm_vars !== "undefined") && (__pm_vars[k] !== undefined);
      },
      set: function (k, v) {
        // keep the in-context view consistent so later get() sees the new value
        if (typeof __pm_vars !== "undefined") __pm_vars[k] = String(v);
        __pm_mutate("set", scopeName, String(k), String(v));
      },
      unset: function (k) {
        if (typeof __pm_vars !== "undefined") delete __pm_vars[k];
        __pm_mutate("unset", scopeName, String(k), null);
      },
    };
  }
  pm.environment = makeScope("environment");
  pm.globals = makeScope("globals");
  pm.collectionVariables = makeScope("collection");
  pm.variables = makeScope("variables");

  // ---- pm.info + pm.iterationData ------------------------------------------
  pm.info = (typeof __pm_info !== "undefined") ? __pm_info : {};
  (function () {
    var d = (typeof __pm_data !== "undefined") ? __pm_data : {};
    pm.iterationData = {
      get: function (k) { return d[k]; },
      has: function (k) { return Object.prototype.hasOwnProperty.call(d, k); },
      toObject: function () { return d; },
    };
  })();

  // ---- pm.execution.setNextRequest + legacy postman.* ----------------------
  function _setNext(name) { __pm_set_next(name == null ? null : String(name)); }
  pm.execution = { setNextRequest: _setNext };
  globalThis.postman = {
    setNextRequest: _setNext,
    setEnvironmentVariable: function (k, v) { pm.environment.set(k, v); },
    getEnvironmentVariable: function (k) { return pm.environment.get(k); },
    setGlobalVariable: function (k, v) { pm.globals.set(k, v); },
    getGlobalVariable: function (k) { return pm.globals.get(k); },
  };

  // ---- pm.cookies (v1: read-only jar built from Set-Cookie on the response) --
  (function () {
    var jar = {};
    if (typeof __pm_response !== "undefined" && __pm_response.__headers) {
      var sc = __pm_response.__headers["set-cookie"];
      if (sc) {
        String(sc).split(",").forEach(function (part) {
          var nameVal = part.split(";")[0];
          var eq = nameVal.indexOf("=");
          if (eq > 0) jar[nameVal.slice(0, eq).trim()] = nameVal.slice(eq + 1).trim();
        });
      }
    }
    pm.cookies = {
      has: function (name) { return Object.prototype.hasOwnProperty.call(jar, name); },
      get: function (name) { return jar[name]; },
    };
  })();

  // ---- pm.sendRequest(reqOrUrl, callback) — synchronous bridge --------------
  pm.sendRequest = function (spec, cb) {
    // __pm_send returns a JSON string: { code, text, ... } or { __error }
    var raw = JSON.parse(__pm_send(spec));
    if (raw && raw.__error) {
      if (cb) cb(new Error(String(raw.__error)), null);
      return;
    }
    var res = {
      code: raw.code,
      status: raw.status,
      responseTime: raw.responseTime,
      text: function () { return raw.text; },
      json: function () { return JSON.parse(raw.text); },
    };
    if (cb) cb(null, res);
  };
})();
