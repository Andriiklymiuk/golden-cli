//! JS shim sources for require(lodash|crypto-js|uuid). Subset-only; the registry
//! throws for unknown modules.

/// lodash subset: get, isEmpty, isNil, pick, map, find, cloneDeep.
pub const LODASH: &str = r#"
({
  get: function (obj, path, def) {
    var parts = Array.isArray(path) ? path : String(path).split('.');
    var cur = obj;
    for (var i = 0; i < parts.length; i++) {
      if (cur == null) return def;
      cur = cur[parts[i]];
    }
    return cur === undefined ? def : cur;
  },
  isNil: function (v) { return v === null || v === undefined; },
  isEmpty: function (v) {
    if (v == null) return true;
    if (Array.isArray(v) || typeof v === 'string') return v.length === 0;
    if (typeof v === 'object') return Object.keys(v).length === 0;
    return false;
  },
  pick: function (obj, keys) {
    var out = {};
    (keys || []).forEach(function (k) { if (k in obj) out[k] = obj[k]; });
    return out;
  },
  map: function (coll, fn) { return (coll || []).map(fn); },
  find: function (coll, fn) { return (coll || []).find(fn); },
  cloneDeep: function (v) { return JSON.parse(JSON.stringify(v)); }
})
"#;

/// crypto-js subset: MD5, SHA256, HmacSHA256 (results have a `.toString(enc)`
/// defaulting to hex; pass `enc.Base64` for base64) plus enc.Hex/Base64/Utf8.
/// Pure-JS implementations evaluated in-context.
pub const CRYPTO_JS: &str = r#"
(function () {
  function toHex(bytes) {
    var s = '';
    for (var i = 0; i < bytes.length; i++) {
      var h = (bytes[i] & 0xff).toString(16);
      s += h.length === 1 ? '0' + h : h;
    }
    return s;
  }
  // Minimal MD5 (RFC 1321) over a UTF-8 string -> 16 bytes.
  function md5bytes(str) {
    function rol(n, c) { return (n << c) | (n >>> (32 - c)); }
    function add(a, b) { return (a + b) & 0xffffffff; }
    var s = [7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,
             5,9,14,20,5,9,14,20,5,9,14,20,5,9,14,20,
             4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,
             6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21];
    var K = [];
    for (var i = 0; i < 64; i++) K[i] = Math.floor(Math.abs(Math.sin(i + 1)) * 4294967296) & 0xffffffff;
    var bytes = [];
    for (var i = 0; i < str.length; i++) {
      var c = str.charCodeAt(i);
      if (c < 128) bytes.push(c);
      else if (c < 2048) { bytes.push(192 | (c >> 6), 128 | (c & 63)); }
      else { bytes.push(224 | (c >> 12), 128 | ((c >> 6) & 63), 128 | (c & 63)); }
    }
    var origLen = bytes.length;
    bytes.push(0x80);
    while (bytes.length % 64 !== 56) bytes.push(0);
    var bitLen = origLen * 8;
    for (var i = 0; i < 8; i++) { bytes.push(bitLen & 0xff); bitLen = Math.floor(bitLen / 256); }
    var a0 = 0x67452301, b0 = 0xefcdab89, c0 = 0x98badcfe, d0 = 0x10325476;
    for (var off = 0; off < bytes.length; off += 64) {
      var M = [];
      for (var j = 0; j < 16; j++) {
        M[j] = bytes[off+j*4] | (bytes[off+j*4+1]<<8) | (bytes[off+j*4+2]<<16) | (bytes[off+j*4+3]<<24);
      }
      var A=a0, B=b0, C=c0, D=d0;
      for (var i2 = 0; i2 < 64; i2++) {
        var F, g;
        if (i2 < 16) { F = (B & C) | (~B & D); g = i2; }
        else if (i2 < 32) { F = (D & B) | (~D & C); g = (5*i2 + 1) % 16; }
        else if (i2 < 48) { F = B ^ C ^ D; g = (3*i2 + 5) % 16; }
        else { F = C ^ (B | ~D); g = (7*i2) % 16; }
        F = add(add(add(F, A), K[i2]), M[g]);
        A = D; D = C; C = B; B = add(B, rol(F, s[i2]));
      }
      a0 = add(a0, A); b0 = add(b0, B); c0 = add(c0, C); d0 = add(d0, D);
    }
    var out = [];
    [a0,b0,c0,d0].forEach(function (x) {
      out.push(x & 0xff, (x>>>8) & 0xff, (x>>>16) & 0xff, (x>>>24) & 0xff);
    });
    return out;
  }
  function utf8bytes(str) {
    var bytes = [];
    for (var i = 0; i < str.length; i++) {
      var c = str.charCodeAt(i);
      if (c < 128) bytes.push(c);
      else if (c < 2048) { bytes.push(192 | (c >> 6), 128 | (c & 63)); }
      else { bytes.push(224 | (c >> 12), 128 | ((c >> 6) & 63), 128 | (c & 63)); }
    }
    return bytes;
  }
  function toBytes(x) { return (x && x.words) ? x.words : utf8bytes(String(x)); }
  var SHA_K = [
    0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
    0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
    0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
    0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
    0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
    0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
    0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
    0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
  ];
  function sha256bytes(bytes) {
    var H = [0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19];
    var msg = bytes.slice();
    var bitLen = msg.length * 8;
    msg.push(0x80);
    while (msg.length % 64 !== 56) msg.push(0);
    msg.push(0, 0, 0, 0, (bitLen>>>24)&0xff, (bitLen>>>16)&0xff, (bitLen>>>8)&0xff, bitLen&0xff);
    function rotr(n, x) { return (x>>>n) | (x<<(32-n)); }
    for (var off = 0; off < msg.length; off += 64) {
      var w = [];
      for (var i = 0; i < 16; i++) {
        w[i] = ((msg[off+i*4]<<24)|(msg[off+i*4+1]<<16)|(msg[off+i*4+2]<<8)|msg[off+i*4+3]) | 0;
      }
      for (var i = 16; i < 64; i++) {
        var s0 = rotr(7,w[i-15]) ^ rotr(18,w[i-15]) ^ (w[i-15]>>>3);
        var s1 = rotr(17,w[i-2]) ^ rotr(19,w[i-2]) ^ (w[i-2]>>>10);
        w[i] = (w[i-16] + s0 + w[i-7] + s1) | 0;
      }
      var a=H[0],b=H[1],c=H[2],d=H[3],e=H[4],f=H[5],g=H[6],h=H[7];
      for (var i = 0; i < 64; i++) {
        var S1 = rotr(6,e) ^ rotr(11,e) ^ rotr(25,e);
        var ch = (e&f) ^ ((~e)&g);
        var t1 = (h + S1 + ch + SHA_K[i] + w[i]) | 0;
        var S0 = rotr(2,a) ^ rotr(13,a) ^ rotr(22,a);
        var maj = (a&b) ^ (a&c) ^ (b&c);
        var t2 = (S0 + maj) | 0;
        h=g; g=f; f=e; e=(d+t1)|0; d=c; c=b; b=a; a=(t1+t2)|0;
      }
      H[0]=(H[0]+a)|0; H[1]=(H[1]+b)|0; H[2]=(H[2]+c)|0; H[3]=(H[3]+d)|0;
      H[4]=(H[4]+e)|0; H[5]=(H[5]+f)|0; H[6]=(H[6]+g)|0; H[7]=(H[7]+h)|0;
    }
    var out = [];
    for (var i = 0; i < 8; i++) out.push((H[i]>>>24)&0xff,(H[i]>>>16)&0xff,(H[i]>>>8)&0xff,H[i]&0xff);
    return out;
  }
  function hmacSha256bytes(msgBytes, keyBytes) {
    var block = 64;
    if (keyBytes.length > block) keyBytes = sha256bytes(keyBytes);
    keyBytes = keyBytes.slice();
    while (keyBytes.length < block) keyBytes.push(0);
    var ipad = [], opad = [];
    for (var i = 0; i < block; i++) { ipad.push(keyBytes[i]^0x36); opad.push(keyBytes[i]^0x5c); }
    return sha256bytes(opad.concat(sha256bytes(ipad.concat(msgBytes))));
  }
  function b64(bytes) {
    var t = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    var out = '', i = 0;
    for (; i + 2 < bytes.length; i += 3) {
      var n = (bytes[i]<<16) | (bytes[i+1]<<8) | bytes[i+2];
      out += t[(n>>18)&63] + t[(n>>12)&63] + t[(n>>6)&63] + t[n&63];
    }
    var rem = bytes.length - i;
    if (rem === 1) { var n1 = bytes[i]<<16; out += t[(n1>>18)&63] + t[(n1>>12)&63] + '=='; }
    else if (rem === 2) { var n2 = (bytes[i]<<16)|(bytes[i+1]<<8); out += t[(n2>>18)&63] + t[(n2>>12)&63] + t[(n2>>6)&63] + '='; }
    return out;
  }
  var Base64Enc = { __b64: true, stringify: function (h) { return b64(h.words || h); } };
  function hashResult(bytes) {
    return {
      words: bytes,
      toString: function (enc) { return (enc && enc.__b64) ? b64(bytes) : toHex(bytes); },
    };
  }
  return {
    MD5: function (s) { return hashResult(md5bytes(String(s))); },
    SHA256: function (s) { return hashResult(sha256bytes(toBytes(s))); },
    HmacSHA256: function (msg, key) { return hashResult(hmacSha256bytes(toBytes(msg), toBytes(key))); },
    enc: {
      Hex: { stringify: function (h) { return toHex(h.words || h); } },
      Base64: Base64Enc,
      Utf8: { parse: function (s) { return { words: utf8bytes(String(s)) }; } },
    }
  };
})()
"#;

/// uuid subset: v4 (random). Uses Math.random — adequate for test fixtures.
pub const UUID: &str = r#"
({
  v4: function () {
    return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function (c) {
      var r = (Math.random() * 16) | 0;
      var v = c === 'x' ? r : (r & 0x3) | 0x8;
      return v.toString(16);
    });
  }
})
"#;
