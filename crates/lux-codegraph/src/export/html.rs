//! The HTML shell for [`super::to_graph_html`].
//!
//! A single self-contained page: inline CSS, an inline dependency-free canvas
//! force-directed renderer, and a `type="application/json"` data block. No network
//! access, no third-party script — it opens offline straight from disk. The export
//! layer substitutes the `__TOKEN__` placeholders; the graph JSON is injected last
//! into `#graph-data`.

/// The page template. `__GRAPH_DATA__` is replaced with the (`</`-escaped) graph
/// JSON; the `__*_NODES__` / `__EDGE_COUNT__` / `__COMMUNITY_COUNT__` /
/// `__TRUNCATED__` / `__TITLE__` tokens with their values.
pub(super) const GRAPH_HTML_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Code Graph — __TITLE__</title>
<style>
  :root {
    --bg: #0b0c0e;
    --panel: #15171b;
    --border: #24272e;
    --text: #e6e8ec;
    --muted: #8b9099;
    --accent: #6ea8fe;
  }
  * { box-sizing: border-box; }
  html, body { margin: 0; height: 100%; overflow: hidden; }
  body {
    background: var(--bg);
    color: var(--text);
    font: 13px/1.5 ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  }
  #bar {
    position: fixed; top: 0; left: 0; right: 0; z-index: 5;
    display: flex; align-items: center; gap: 14px;
    padding: 10px 16px;
    background: linear-gradient(180deg, rgba(11,12,14,.95), rgba(11,12,14,.7));
    border-bottom: 1px solid var(--border);
    backdrop-filter: blur(6px);
  }
  #bar h1 { font-size: 14px; font-weight: 600; margin: 0; white-space: nowrap; }
  #bar .stats { color: var(--muted); font-size: 12px; white-space: nowrap; }
  #bar .stats b { color: var(--text); font-weight: 600; }
  #bar .notice { color: #f0b95b; font-size: 12px; }
  #bar .spacer { flex: 1; }
  #search {
    background: var(--panel); border: 1px solid var(--border); color: var(--text);
    border-radius: 6px; padding: 5px 10px; width: 200px; outline: none;
  }
  #search:focus { border-color: var(--accent); }
  #reset {
    background: var(--panel); border: 1px solid var(--border); color: var(--text);
    border-radius: 6px; padding: 5px 12px; cursor: pointer;
  }
  #reset:hover { border-color: var(--accent); }
  canvas { display: block; position: fixed; inset: 0; }
  #tip {
    position: fixed; z-index: 6; pointer-events: none; display: none;
    max-width: 320px; padding: 8px 10px;
    background: var(--panel); border: 1px solid var(--border); border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0,0,0,.4); font-size: 12px;
  }
  #tip .t-label { font-weight: 600; }
  #tip .t-meta { color: var(--muted); margin-top: 2px; }
  #legend {
    position: fixed; bottom: 14px; left: 14px; z-index: 5;
    background: rgba(21,23,27,.85); border: 1px solid var(--border);
    border-radius: 8px; padding: 10px 12px; max-width: 240px;
  }
  #legend .head { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: .04em; margin-bottom: 6px; }
  #legend .row { display: flex; align-items: center; gap: 8px; margin: 3px 0; }
  #legend .dot { width: 10px; height: 10px; border-radius: 50%; flex: none; }
  #hint { position: fixed; bottom: 14px; right: 14px; z-index: 5; color: var(--muted); font-size: 11px; text-align: right; }
</style>
</head>
<body>
  <div id="bar">
    <h1>Code Graph — __TITLE__</h1>
    <span class="stats"><b>__SHOWN_NODES__</b> nodes · <b>__EDGE_COUNT__</b> edges · <b>__COMMUNITY_COUNT__</b> communities</span>
    <span class="notice" id="notice"></span>
    <span class="spacer"></span>
    <input id="search" type="search" placeholder="Find a symbol…" autocomplete="off" />
    <button id="reset">Reset view</button>
  </div>
  <canvas id="g"></canvas>
  <div id="tip"></div>
  <div id="legend"><div class="head">Communities</div><div id="legend-rows"></div></div>
  <div id="hint">drag node · drag bg to pan · wheel to zoom · click to focus</div>

  <script type="application/json" id="graph-data">__GRAPH_DATA__</script>
  <script>
  (function () {
    "use strict";
    var TRUNCATED = __TRUNCATED__, TOTAL = __TOTAL_NODES__, SHOWN = __SHOWN_NODES__;
    var raw = JSON.parse(document.getElementById("graph-data").textContent);

    var PALETTE = ["#6ea8fe","#f783ac","#63e6be","#ffd43b","#b197fc","#ff922b",
                   "#74c0fc","#ffa8a8","#8ce99a","#ffe066","#e599f7","#ffc078",
                   "#4dabf7","#faa2c1","#69db7c","#a9e34b"];
    function colorFor(c) { return (c === null || c === undefined) ? "#5c6370" : PALETTE[((c % PALETTE.length) + PALETTE.length) % PALETTE.length]; }

    // Build node objects with positions; index by id.
    var byId = Object.create(null);
    var nodes = raw.nodes.map(function (n, i) {
      var ang = (i / raw.nodes.length) * Math.PI * 2;
      var o = {
        id: n.id, label: n.label || "", type: n.type || "symbol", file: n.file || "",
        community: (n.community === undefined ? null : n.community), degree: n.degree || 0,
        x: Math.cos(ang) * 240 + (Math.random() - 0.5) * 60,
        y: Math.sin(ang) * 240 + (Math.random() - 0.5) * 60,
        vx: 0, vy: 0
      };
      o.r = Math.min(22, 3 + Math.sqrt(o.degree) * 2);
      o.color = colorFor(o.community);
      byId[o.id] = o;
      return o;
    });
    var links = raw.links.map(function (l) {
      return { s: byId[l.source], t: byId[l.target], relation: l.relation, confidence: l.confidence };
    }).filter(function (l) { return l.s && l.t; });

    // Adjacency for focus highlighting.
    var adj = Object.create(null);
    links.forEach(function (l) {
      (adj[l.s.id] = adj[l.s.id] || []).push(l.t.id);
      (adj[l.t.id] = adj[l.t.id] || []).push(l.s.id);
    });

    var canvas = document.getElementById("g"), ctx = canvas.getContext("2d");
    var DPR = Math.max(1, window.devicePixelRatio || 1);
    var W = 0, H = 0;
    function resize() {
      W = window.innerWidth; H = window.innerHeight;
      canvas.width = W * DPR; canvas.height = H * DPR;
      canvas.style.width = W + "px"; canvas.style.height = H + "px";
      ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
    }
    window.addEventListener("resize", function () { resize(); draw(); });

    // Camera.
    var cam = { x: 0, y: 0, scale: 1 };
    cam.x = W; // set after resize below
    function centerCamera() { cam.x = W / 2; cam.y = H / 2; cam.scale = 1; }

    // ── Force simulation ──
    var alpha = 1, REPULSION = 5200, SPRING = 0.02, LINK_LEN = 70, GRAVITY = 0.0022, DAMP = 0.86;
    var dragNode = null;
    function step() {
      if (alpha < 0.02 && !dragNode) return false;
      var n = nodes.length, i, j;
      for (i = 0; i < n; i++) {
        var a = nodes[i];
        for (j = i + 1; j < n; j++) {
          var b = nodes[j];
          var dx = a.x - b.x, dy = a.y - b.y;
          var d2 = dx * dx + dy * dy; if (d2 < 1) d2 = 1;
          var f = REPULSION / d2;
          var d = Math.sqrt(d2);
          var fx = (dx / d) * f, fy = (dy / d) * f;
          a.vx += fx; a.vy += fy; b.vx -= fx; b.vy -= fy;
        }
        a.vx -= a.x * GRAVITY; a.vy -= a.y * GRAVITY;
      }
      for (i = 0; i < links.length; i++) {
        var l = links[i], dx2 = l.t.x - l.s.x, dy2 = l.t.y - l.s.y;
        var dist = Math.sqrt(dx2 * dx2 + dy2 * dy2) || 1;
        var force = (dist - LINK_LEN) * SPRING;
        var ux = (dx2 / dist) * force, uy = (dy2 / dist) * force;
        l.s.vx += ux; l.s.vy += uy; l.t.vx -= ux; l.t.vy -= uy;
      }
      for (i = 0; i < n; i++) {
        var p = nodes[i];
        if (p === dragNode) { p.vx = 0; p.vy = 0; continue; }
        p.vx *= DAMP; p.vy *= DAMP;
        p.x += p.vx * alpha; p.y += p.vy * alpha;
      }
      alpha *= 0.985;
      return true;
    }

    // ── Rendering ──
    var focus = null, hover = null;
    function confAlpha(c) { return c === "EXTRACTED" ? 0.55 : (c === "INFERRED" ? 0.3 : 0.16); }
    function visible(id) {
      if (!focus) return true;
      if (id === focus.id) return true;
      var ns = adj[focus.id]; return ns && ns.indexOf(id) !== -1;
    }
    function draw() {
      ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
      ctx.clearRect(0, 0, W, H);
      ctx.save();
      ctx.translate(cam.x, cam.y); ctx.scale(cam.scale, cam.scale);

      // Edges.
      for (var i = 0; i < links.length; i++) {
        var l = links[i];
        var on = !focus || (visible(l.s.id) && visible(l.t.id) && (l.s.id === focus.id || l.t.id === focus.id));
        ctx.strokeStyle = "rgba(150,160,175," + (focus ? (on ? confAlpha(l.confidence) + 0.25 : 0.04) : confAlpha(l.confidence)) + ")";
        ctx.lineWidth = (focus && on ? 1.4 : 0.7) / cam.scale;
        ctx.beginPath(); ctx.moveTo(l.s.x, l.s.y); ctx.lineTo(l.t.x, l.t.y); ctx.stroke();
      }
      // Nodes.
      for (i = 0; i < nodes.length; i++) {
        var p = nodes[i], dim = focus && !visible(p.id);
        ctx.globalAlpha = dim ? 0.18 : 1;
        ctx.beginPath(); ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
        ctx.fillStyle = p.color; ctx.fill();
        if (p === hover || (focus && p.id === focus.id)) {
          ctx.lineWidth = 2 / cam.scale; ctx.strokeStyle = "#fff"; ctx.stroke();
        }
        // Labels for hubs, hovered, or focused neighborhood.
        if (!dim && (p.degree >= 6 || p === hover || (focus && visible(p.id)))) {
          ctx.globalAlpha = dim ? 0.18 : 0.9;
          ctx.fillStyle = "#cfd3da";
          ctx.font = (11 / cam.scale) + "px ui-sans-serif, system-ui, sans-serif";
          ctx.fillText(p.label, p.x + p.r + 3 / cam.scale, p.y + 3 / cam.scale);
        }
        ctx.globalAlpha = 1;
      }
      ctx.restore();
    }

    function loop() { var moved = step(); draw(); if (moved) requestAnimationFrame(loop); else running = false; }
    var running = false;
    function kick() { alpha = Math.max(alpha, 0.5); if (!running) { running = true; requestAnimationFrame(loop); } }

    // ── Interaction ──
    function toWorld(sx, sy) { return { x: (sx - cam.x) / cam.scale, y: (sy - cam.y) / cam.scale }; }
    function pick(sx, sy) {
      var w = toWorld(sx, sy), best = null, bd = Infinity;
      for (var i = 0; i < nodes.length; i++) {
        var p = nodes[i], dx = p.x - w.x, dy = p.y - w.y, d = dx * dx + dy * dy;
        var rr = (p.r + 4) * (p.r + 4) / (cam.scale * cam.scale);
        if (d <= rr && d < bd) { bd = d; best = p; }
      }
      return best;
    }
    var down = null, dragged = false, panStart = null;
    canvas.addEventListener("mousedown", function (e) {
      down = { x: e.clientX, y: e.clientY }; dragged = false;
      var hit = pick(e.clientX, e.clientY);
      if (hit) { dragNode = hit; kick(); }
      else { panStart = { x: e.clientX - cam.x, y: e.clientY - cam.y }; }
    });
    window.addEventListener("mousemove", function (e) {
      if (down && (Math.abs(e.clientX - down.x) + Math.abs(e.clientY - down.y) > 3)) dragged = true;
      if (dragNode) {
        var w = toWorld(e.clientX, e.clientY); dragNode.x = w.x; dragNode.y = w.y; dragNode.vx = 0; dragNode.vy = 0; kick();
      } else if (panStart) {
        cam.x = e.clientX - panStart.x; cam.y = e.clientY - panStart.y; draw();
      } else {
        var hit = pick(e.clientX, e.clientY);
        if (hit !== hover) { hover = hit; draw(); }
        var tip = document.getElementById("tip");
        if (hit) {
          tip.style.display = "block";
          tip.style.left = (e.clientX + 14) + "px"; tip.style.top = (e.clientY + 14) + "px";
          tip.innerHTML = '<div class="t-label"></div><div class="t-meta"></div>';
          tip.querySelector(".t-label").textContent = hit.label;
          tip.querySelector(".t-meta").textContent = hit.type + " · degree " + hit.degree + (hit.file ? " · " + hit.file : "");
        } else { tip.style.display = "none"; }
      }
    });
    window.addEventListener("mouseup", function (e) {
      if (!dragged) {
        var hit = pick(e.clientX, e.clientY);
        focus = (hit && (!focus || focus.id !== hit.id)) ? hit : null;
        draw();
      }
      down = null; dragNode = null; panStart = null;
    });
    canvas.addEventListener("wheel", function (e) {
      e.preventDefault();
      var w = toWorld(e.clientX, e.clientY);
      var factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
      cam.scale = Math.min(4, Math.max(0.1, cam.scale * factor));
      cam.x = e.clientX - w.x * cam.scale; cam.y = e.clientY - w.y * cam.scale;
      draw();
    }, { passive: false });

    document.getElementById("reset").addEventListener("click", function () {
      focus = null; centerCamera(); kick();
    });
    document.getElementById("search").addEventListener("input", function (e) {
      var q = e.target.value.trim().toLowerCase();
      if (!q) { focus = null; draw(); return; }
      var found = nodes.find(function (p) { return p.label.toLowerCase().indexOf(q) !== -1; });
      if (found) { focus = found; cam.scale = Math.max(cam.scale, 1.2); cam.x = W / 2 - found.x * cam.scale; cam.y = H / 2 - found.y * cam.scale; draw(); }
    });

    // Legend (top communities by node count).
    (function () {
      var counts = {};
      nodes.forEach(function (p) { var k = p.community === null ? "—" : p.community; counts[k] = (counts[k] || 0) + 1; });
      var rows = Object.keys(counts).sort(function (a, b) { return counts[b] - counts[a]; }).slice(0, 10);
      var html = "";
      rows.forEach(function (k) {
        var c = k === "—" ? null : Number(k);
        html += '<div class="row"><span class="dot" style="background:' + colorFor(c) + '"></span>' +
                (k === "—" ? "unassigned" : "Community " + k) + ' <span style="color:var(--muted)">(' + counts[k] + ")</span></div>";
      });
      document.getElementById("legend-rows").innerHTML = html;
    })();

    if (TRUNCATED) document.getElementById("notice").textContent = "showing top " + SHOWN + " of " + TOTAL + " nodes";

    resize(); centerCamera(); kick();
  })();
  </script>
</body>
</html>
"##;
