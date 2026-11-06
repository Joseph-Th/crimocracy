(() => {
  "use strict";

  const $ = (id) => document.getElementById(id);
  const canvas = $("city-canvas");
  const errorBox = $("fatal-error");
  const meshColors = [
    [0.42, 0.51, 0.55], // residential
    [0.69, 0.54, 0.36], // storefront
    [0.48, 0.59, 0.60], // office
    [0.49, 0.46, 0.42], // industrial
    [0.59, 0.39, 0.36], // leisure
    [0.38, 0.47, 0.51], // garage
    [0.22, 0.26, 0.27], // road
    [0.29, 0.34, 0.32], // district ground
  ];
  const streetMesh = 6;
  const groundMesh = 7;
  const stride = 32;

  function fail(message) {
    errorBox.hidden = false;
    errorBox.textContent = message;
    $("render-status").textContent = "RENDERER UNAVAILABLE";
    document.documentElement.dataset.webgl = "unavailable";
  }

  let scene;
  try {
    const embeddedData = $("city-preview-data");
    scene = JSON.parse(embeddedData.textContent);
    // The parsed scene supersedes the large inlined JSON text in the DOM.
    embeddedData.remove();
  } catch (error) {
    fail("The city export could not be loaded: " + error.message);
    return;
  }
  if (!Array.isArray(scene.districts) || scene.districts.length === 0 || !Array.isArray(scene.dossiers)) {
    fail("The city export does not contain any renderable districts or a public directory.");
    return;
  }

  const gl = canvas.getContext("webgl2", {
    alpha: false,
    antialias: true,
    depth: true,
    stencil: false,
    preserveDrawingBuffer: false,
  });
  if (!gl) {
    fail("WebGL2 is unavailable. This preview requires a browser with a working WebGL2 implementation.");
    return;
  }
  gl.enable(gl.DEPTH_TEST);
  gl.depthFunc(gl.LEQUAL);
  gl.disable(gl.BLEND);
  gl.disable(gl.DITHER);

  const vertexSource = `#version 300 es
    precision highp float;
    precision highp int;
    layout(location=0) in vec3 aPosition;
    layout(location=1) in vec3 aNormal;
    layout(location=2) in vec3 aTranslation;
    layout(location=3) in vec3 aScale;
    layout(location=4) in uint aPickId;
    layout(location=5) in uint aVariation;
    uniform vec3 uFocus;
    uniform vec3 uRight;
    uniform vec3 uUp;
    uniform vec3 uTowardEye;
    uniform vec2 uHalfExtents;
    out vec3 vNormal;
    out vec3 vLocal;
    out vec3 vDimensions;
    flat out uint vPickId;
    flat out uint vVariation;
    void main() {
      vec3 point = aPosition * aScale + aTranslation;
      vec3 delta = point - uFocus;
      gl_Position = vec4(
        dot(delta, uRight) / uHalfExtents.x,
        dot(delta, uUp) / uHalfExtents.y,
        -dot(delta, uTowardEye) / 16000.0,
        1.0
      );
      vNormal = aNormal;
      vLocal = aPosition;
      vDimensions = aScale;
      vPickId = aPickId;
      vVariation = aVariation;
    }
  `;
  const fragmentSource = `#version 300 es
    precision highp float;
    precision highp int;
    in vec3 vNormal;
    in vec3 vLocal;
    in vec3 vDimensions;
    flat in uint vPickId;
    flat in uint vVariation;
    uniform int uMeshClass;
    uniform bool uPicking;
    uniform uint uSelected;
    uniform vec2 uHalfExtents;
    uniform vec3 uPalette[8];
    out vec4 outColor;
    void main() {
      if (uPicking) {
        outColor = vec4(
          float((vPickId >> 0u) & 255u) / 255.0,
          float((vPickId >> 8u) & 255u) / 255.0,
          float((vPickId >> 16u) & 255u) / 255.0,
          float((vPickId >> 24u) & 255u) / 255.0
        );
        return;
      }
      vec3 base = uPalette[uMeshClass];
      float light = 0.57 + 0.43 * max(dot(normalize(vNormal), normalize(vec3(-0.43, 0.84, 0.39))), 0.0);
      float variation = float(vVariation % 31u) / 31.0;
      base *= (0.85 + variation * 0.28) * light;
      // Draw facade windows only at close range. They require no extra mesh,
      // texture lookup, or draw call and disappear before subpixel aliasing.
      if (uMeshClass < 6 && uHalfExtents.y < 350.0
          && abs(vNormal.y) < 0.5 && vDimensions.y >= 7.0) {
        float facadeX = abs(vNormal.z) > 0.5 ? vLocal.x : vLocal.z;
        float facadeWidth = abs(vNormal.z) > 0.5 ? vDimensions.x : vDimensions.z;
        float columns = max(1.0, floor(facadeWidth / 3.7));
        float floors = max(1.0, floor(vDimensions.y / 3.5));
        vec2 pane = fract(vec2((facadeX + 0.5) * columns, (vLocal.y + 0.5) * floors));
        if (pane.x > 0.20 && pane.x < 0.76 && pane.y > 0.27 && pane.y < 0.72) {
          float warm = float((vVariation >> 3u) & 1u) * 0.28;
          base = mix(base, vec3(0.10, 0.16, 0.19) + warm * vec3(1.0, 0.74, 0.38), 0.78);
        }
      }
      if (vPickId != 0u) {
        base = mix(base, vec3(0.83, 0.71, 0.48), 0.16);
      }
      if (vPickId != 0u && vPickId == uSelected) {
        base = mix(base, vec3(0.99, 0.79, 0.38), 0.61);
      }
      outColor = vec4(base, 1.0);
    }
  `;

  function shader(kind, source) {
    const handle = gl.createShader(kind);
    gl.shaderSource(handle, source);
    gl.compileShader(handle);
    if (!gl.getShaderParameter(handle, gl.COMPILE_STATUS)) {
      throw new Error("Shader compilation failed: " + gl.getShaderInfoLog(handle));
    }
    return handle;
  }
  function program() {
    const handle = gl.createProgram();
    const vs = shader(gl.VERTEX_SHADER, vertexSource);
    const fs = shader(gl.FRAGMENT_SHADER, fragmentSource);
    gl.attachShader(handle, vs);
    gl.attachShader(handle, fs);
    gl.linkProgram(handle);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    if (!gl.getProgramParameter(handle, gl.LINK_STATUS)) {
      throw new Error("Shader linking failed: " + gl.getProgramInfoLog(handle));
    }
    return handle;
  }

  let shaderProgram;
  try {
    shaderProgram = program();
  } catch (error) {
    fail(error.message);
    return;
  }

  const uniforms = {
    focus: gl.getUniformLocation(shaderProgram, "uFocus"),
    right: gl.getUniformLocation(shaderProgram, "uRight"),
    up: gl.getUniformLocation(shaderProgram, "uUp"),
    towardEye: gl.getUniformLocation(shaderProgram, "uTowardEye"),
    halfExtents: gl.getUniformLocation(shaderProgram, "uHalfExtents"),
    mesh: gl.getUniformLocation(shaderProgram, "uMeshClass"),
    picking: gl.getUniformLocation(shaderProgram, "uPicking"),
    selected: gl.getUniformLocation(shaderProgram, "uSelected"),
    palette: gl.getUniformLocation(shaderProgram, "uPalette[0]"),
  };
  gl.useProgram(shaderProgram);
  gl.uniform3fv(uniforms.palette, new Float32Array(meshColors.flat()));

  function cubeVertices() {
    const vertices = [];
    const faces = [
      { n: [0, 1, 0], corners: [[-1, 1, -1], [-1, 1, 1], [1, 1, 1], [1, 1, -1]] },
      { n: [0, -1, 0], corners: [[-1, -1, 1], [-1, -1, -1], [1, -1, -1], [1, -1, 1]] },
      { n: [1, 0, 0], corners: [[1, -1, -1], [1, 1, -1], [1, 1, 1], [1, -1, 1]] },
      { n: [-1, 0, 0], corners: [[-1, -1, 1], [-1, 1, 1], [-1, 1, -1], [-1, -1, -1]] },
      { n: [0, 0, 1], corners: [[1, -1, 1], [1, 1, 1], [-1, 1, 1], [-1, -1, 1]] },
      { n: [0, 0, -1], corners: [[-1, -1, -1], [-1, 1, -1], [1, 1, -1], [1, -1, -1]] },
    ];
    for (const face of faces) {
      for (const i of [0, 1, 2, 0, 2, 3]) {
        vertices.push(...face.corners[i].map((value) => value * 0.5), ...face.n);
      }
    }
    return new Float32Array(vertices);
  }
  const cubeBuffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, cubeBuffer);
  gl.bufferData(gl.ARRAY_BUFFER, cubeVertices(), gl.STATIC_DRAW);

  function decodeBase64(base64) {
    const raw = atob(base64);
    const bytes = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
    return bytes;
  }
  function packedInstance(translation, scale, pickId = 0, variation = 0) {
    const bytes = new Uint8Array(stride);
    const view = new DataView(bytes.buffer);
    for (let i = 0; i < 3; i++) view.setFloat32(i * 4, translation[i], true);
    for (let i = 0; i < 3; i++) view.setFloat32((i + 3) * 4, scale[i], true);
    view.setUint32(24, pickId, true);
    view.setUint32(28, variation, true);
    return bytes;
  }
  function configureInstanceAttributes(startOffset) {
    gl.vertexAttribPointer(2, 3, gl.FLOAT, false, stride, startOffset);
    gl.vertexAttribPointer(3, 3, gl.FLOAT, false, stride, startOffset + 12);
    gl.vertexAttribIPointer(4, 1, gl.UNSIGNED_INT, stride, startOffset + 24);
    gl.vertexAttribIPointer(5, 1, gl.UNSIGNED_INT, stride, startOffset + 28);
  }
  function makeGpuBatch(encoded, mesh, count) {
    if (count === 0) return null;
    if (encoded.byteLength !== count * stride) throw new Error("Instance buffer size mismatch for mesh " + mesh);
    const instanceBuffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, instanceBuffer);
    gl.bufferData(gl.ARRAY_BUFFER, encoded, gl.STATIC_DRAW);
    const vao = gl.createVertexArray();
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, cubeBuffer);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 24, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 3, gl.FLOAT, false, 24, 12);
    gl.bindBuffer(gl.ARRAY_BUFFER, instanceBuffer);
    for (const location of [2, 3, 4, 5]) {
      gl.enableVertexAttribArray(location);
      gl.vertexAttribDivisor(location, 1);
    }
    configureInstanceAttributes(0);
    gl.bindVertexArray(null);
    return { vao, instanceBuffer, count, mesh };
  }

  // Retain the encoded buffers of offscreen districts. Convert and upload a
  // chunk only when the camera reaches it, then release its base64 payload.
  const gpuDistricts = scene.districts.map((district) => {
      const prepared = {
        id: district.id,
        name: district.name,
        bounds: district.bounds,
        sectors: district.sectors,
        street_count: district.street_count,
        encodedStreets: district.streets,
        encodedBuildings: district.buildings,
        gpu: null,
      };
      district.streets = null;
      district.buildings = null;
      return prepared;
  });
  let uploadedDistricts = 0;
  function ensureGpu(district) {
    if (district.gpu) return district.gpu;
    const cx = (district.bounds[0] + district.bounds[2]) / 2;
    const cz = (district.bounds[1] + district.bounds[3]) / 2;
    const ground = makeGpuBatch(
      packedInstance([cx, -0.08, cz], [district.bounds[2] - district.bounds[0] - 10, 0.16, district.bounds[3] - district.bounds[1] - 10]),
      groundMesh, 1
    );
    const streets = makeGpuBatch(
      decodeBase64(district.encodedStreets), streetMesh, district.street_count
    );
    const buildings = district.encodedBuildings.map((batch) => makeGpuBatch(
      decodeBase64(batch.bytes), batch.mesh, batch.instance_count
    ));
    district.gpu = { ground, streets, buildings };
    district.encodedStreets = null;
    district.encodedBuildings = null;
    uploadedDistricts++;
    return district.gpu;
  }

  let pickingTarget = null;
  function resize() {
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    const width = Math.max(1, Math.round(canvas.clientWidth * ratio));
    const height = Math.max(1, Math.round(canvas.clientHeight * ratio));
    if (canvas.width === width && canvas.height === height) return;
    canvas.width = width;
    canvas.height = height;
    if (pickingTarget) {
      gl.deleteTexture(pickingTarget.texture);
      gl.deleteRenderbuffer(pickingTarget.depth);
      gl.deleteFramebuffer(pickingTarget.framebuffer);
    }
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    const depth = gl.createRenderbuffer();
    gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT16, width, height);
    const framebuffer = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
      throw new Error("Could not create building-picking framebuffer");
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    pickingTarget = { texture, depth, framebuffer };
    gl.viewport(0, 0, width, height);
  }

  const firstDistrict = scene.districts[0];
  const startFocus = [
    (firstDistrict.bounds[0] + firstDistrict.bounds[2]) / 2,
    (firstDistrict.bounds[1] + firstDistrict.bounds[3]) / 2,
  ];
  const camera = { x: startFocus[0], z: startFocus[1], yaw: Math.PI / 4, elevation: 0.89, zoom: 440 };
  let selectedId = 0;
  let selectedWard = firstDistrict.id;
  const properties = new Map(scene.dossiers.map((entry) => [entry.id >>> 0, entry]));
  const labelNodes = new Map();
  const wardLabelNodes = new Map();
  const visibleStats = { buildings: 0, drawCalls: 0 };

  function basis() {
    const c = Math.cos(camera.yaw);
    const s = Math.sin(camera.yaw);
    const e = Math.sin(camera.elevation);
    const ce = Math.cos(camera.elevation);
    return {
      right: [c, 0, -s],
      up: [-s * e, ce, -c * e],
      eye: [s * ce, e, c * ce],
      halfHeight: camera.zoom,
      halfWidth: camera.zoom * canvas.width / canvas.height,
    };
  }
  function worldToScreen(position, axes) {
    const dx = position[0] - camera.x;
    const dy = position[1];
    const dz = position[2] - camera.z;
    const sx = dx * axes.right[0] + dy * axes.right[1] + dz * axes.right[2];
    const sy = dx * axes.up[0] + dy * axes.up[1] + dz * axes.up[2];
    return [
      (0.5 + sx / (2 * axes.halfWidth)) * canvas.clientWidth,
      (0.5 - sy / (2 * axes.halfHeight)) * canvas.clientHeight,
    ];
  }
  function groundFromScreen(screenX, screenY, axes) {
    const sx = (screenX / canvas.clientWidth * 2 - 1) * axes.halfWidth;
    const sy = (1 - screenY / canvas.clientHeight * 2) * axes.halfHeight;
    const determinant = axes.right[0] * axes.up[2] - axes.right[2] * axes.up[0];
    const x = (sx * axes.up[2] - axes.right[2] * sy) / determinant;
    const z = (axes.right[0] * sy - axes.up[0] * sx) / determinant;
    return [camera.x + x, camera.z + z];
  }
  function worldViewport(axes) {
    const corners = [
      groundFromScreen(0, 0, axes),
      groundFromScreen(canvas.clientWidth, 0, axes),
      groundFromScreen(canvas.clientWidth, canvas.clientHeight, axes),
      groundFromScreen(0, canvas.clientHeight, axes),
    ];
    const buffer = 100;
    return [
      Math.min(...corners.map((c) => c[0])) - buffer,
      Math.min(...corners.map((c) => c[1])) - buffer,
      Math.max(...corners.map((c) => c[0])) + buffer,
      Math.max(...corners.map((c) => c[1])) + buffer,
    ];
  }
  function intersects(a, b) {
    return a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1];
  }

  function drawBatch(gpu, start, count, picking) {
    if (!gpu || count === 0) return;
    gl.uniform1i(uniforms.mesh, gpu.mesh);
    gl.bindVertexArray(gpu.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, gpu.instanceBuffer);
    configureInstanceAttributes(start * stride);
    gl.drawArraysInstanced(gl.TRIANGLES, 0, 36, count);
    visibleStats.drawCalls++;
    if (gpu.mesh < 6 && !picking) visibleStats.buildings += count;
  }

  function drawPass(picking) {
    resize();
    const axes = basis();
    const viewport = worldViewport(axes);
    const sectorMode = camera.zoom < 680;
    $("map-detail").textContent = sectorMode ? "SECTOR CULLING" : "DISTRICT CULLING";
    const fb = picking ? pickingTarget.framebuffer : null;
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(picking ? 0 : 0.065, picking ? 0 : 0.10, picking ? 0 : 0.115, picking ? 0 : 1);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.useProgram(shaderProgram);
    gl.uniform3fv(uniforms.focus, [camera.x, 0, camera.z]);
    gl.uniform3fv(uniforms.right, axes.right);
    gl.uniform3fv(uniforms.up, axes.up);
    gl.uniform3fv(uniforms.towardEye, axes.eye);
    gl.uniform2f(uniforms.halfExtents, axes.halfWidth, axes.halfHeight);
    gl.uniform1i(uniforms.picking, picking ? 1 : 0);
    gl.uniform1ui(uniforms.selected, selectedId >>> 0);
    visibleStats.buildings = 0;
    visibleStats.drawCalls = 0;
    for (const district of gpuDistricts) {
      if (!intersects(district.bounds, viewport)) continue;
      const gpu = ensureGpu(district);
      drawBatch(gpu.ground, 0, 1, picking);
      drawBatch(gpu.streets, 0, district.street_count, picking);
      for (let index = 0; index < gpu.buildings.length; index++) {
        const batch = gpu.buildings[index];
        if (!batch) continue;
        if (!sectorMode) {
          drawBatch(batch, 0, batch.count, picking);
          continue;
        }
        let first = -1;
        let count = 0;
        const flush = () => {
          if (count !== 0) drawBatch(batch, first, count, picking);
          first = -1;
          count = 0;
        };
        for (const sector of district.sectors) {
          if (!intersects(sector.bounds, viewport)) continue;
          const range = sector.ranges[batch.mesh];
          if (range[1] === 0) continue;
          if (first >= 0 && first + count === range[0]) {
            count += range[1];
          } else {
            flush();
            first = range[0];
            count = range[1];
          }
        }
        flush();
      }
    }
    gl.bindVertexArray(null);
    if (!picking) {
      $("visible-count").textContent = visibleStats.buildings.toLocaleString();
      $("draw-count").textContent = visibleStats.drawCalls.toLocaleString();
      $("render-status").textContent = "WEBGL2 ACTIVE";
      updateLabels(axes);
      $("compass-arrow").style.transform = `rotate(${-camera.yaw * 180 / Math.PI}deg)`;
      document.documentElement.dataset.webgl = "ready";
    }
  }

  let scheduled = false;
  function requestDraw() {
    if (scheduled) return;
    scheduled = true;
    window.requestAnimationFrame(() => {
      scheduled = false;
      try {
        drawPass(false);
      } catch (error) {
        fail("Drawing failed: " + error.message);
      }
    });
  }
  function pick(clientX, clientY) {
    const rect = canvas.getBoundingClientRect();
    const x = Math.floor((clientX - rect.left) / rect.width * canvas.width);
    const y = canvas.height - 1 - Math.floor((clientY - rect.top) / rect.height * canvas.height);
    if (x < 0 || y < 0 || x >= canvas.width || y >= canvas.height) return;
    drawPass(true);
    const rgba = new Uint8Array(4);
    gl.readPixels(x, y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    const id = (rgba[0] | rgba[1] << 8 | rgba[2] << 16 | rgba[3] << 24) >>> 0;
    selectBusiness(properties.has(id) ? id : 0);
  }

  function centerOnWard(wardId) {
    const district = scene.districts.find((district) => district.id === wardId);
    if (!district) return;
    selectedWard = wardId;
    camera.x = (district.bounds[0] + district.bounds[2]) / 2;
    camera.z = (district.bounds[1] + district.bounds[3]) / 2;
    camera.zoom = 440;
    updateWardButtons();
    requestDraw();
  }
  function focusOnProperty(id) {
    const entry = properties.get(id);
    if (!entry) return;
    camera.x = entry.pin[0];
    camera.z = entry.pin[2];
    camera.zoom = 250;
    selectedWard = entry.ward_id;
    updateWardButtons();
    requestDraw();
  }

  function selectBusiness(id) {
    const entry = properties.get(id);
    selectedId = entry ? entry.id >>> 0 : 0;
    const isSelected = !!entry;
    $("dossier").classList.toggle("is-empty", !isSelected);
    $("dossier-empty").hidden = isSelected;
    $("dossier-detail").hidden = !isSelected;
    $("focus-property").hidden = !isSelected;
    if (entry) {
      $("dossier-number").textContent = String(entry.id).padStart(5, "0");
      $("dossier-name").textContent = entry.name;
      $("dossier-kind").textContent = entry.kind;
      $("dossier-ward").textContent = entry.ward;
      selectedWard = entry.ward_id;
    }
    document.documentElement.dataset.selectedBusiness = String(selectedId);
    updatePropertyButtons();
    updateWardButtons();
    requestDraw();
  }

  function makeElement(tag, className, text) {
    const node = document.createElement(tag);
    node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }
  const wardButtons = new Map();
  function updateWardButtons() {
    for (const [id, button] of wardButtons) {
      button.classList.toggle("active", id === selectedWard);
    }
  }
  for (const district of scene.districts) {
    const button = makeElement("button", "ward-button");
    button.type = "button";
    button.append(makeElement("span", "", district.name));
    button.append(makeElement("small", "", String(district.id).padStart(2, "0")));
    button.addEventListener("click", () => centerOnWard(district.id));
    $("ward-list").append(button);
    wardButtons.set(district.id, button);
  }
  $("district-count").textContent = String(scene.districts.length).padStart(2, "0");
  updateWardButtons();

  const propertyButtons = new Map();
  function updatePropertyButtons() {
    for (const [id, button] of propertyButtons) {
      button.classList.toggle("active", id === selectedId);
    }
    for (const [id, label] of labelNodes) {
      label.classList.toggle("selected", id === selectedId);
    }
  }
  function renderDirectory() {
    const query = $("search").value.trim().toLocaleLowerCase();
    const entries = scene.dossiers.filter((entry) =>
      !query || [entry.name, entry.kind, entry.ward].some((word) => word.toLocaleLowerCase().includes(query))
    );
    $("property-list").replaceChildren();
    propertyButtons.clear();
    $("property-count").textContent = entries.length + "/" + scene.dossiers.length;
    if (entries.length === 0) {
      $("property-list").append(makeElement("div", "directory-empty", "No directory entries match your search."));
    }
    for (const entry of entries) {
      const button = makeElement("button", "property-button");
      button.type = "button";
      button.append(makeElement("span", "property-name", entry.name));
      button.append(makeElement("span", "property-kind", entry.kind + "  /  " + entry.ward));
      button.addEventListener("click", () => {
        selectBusiness(entry.id);
        focusOnProperty(entry.id);
      });
      propertyButtons.set(entry.id, button);
      $("property-list").append(button);
    }
    updatePropertyButtons();
  }
  $("search").addEventListener("input", renderDirectory);
  renderDirectory();

  for (const entry of scene.dossiers) {
    const label = makeElement("button", "world-label", entry.name);
    label.type = "button";
    label.addEventListener("click", () => {
      selectBusiness(entry.id);
      focusOnProperty(entry.id);
    });
    $("world-labels").append(label);
    labelNodes.set(entry.id, label);
  }
  for (const district of scene.districts) {
    const label = makeElement("button", "world-ward");
    label.type = "button";
    label.append(makeElement("span", "", district.name));
    label.append(makeElement("small", "", "FOCUS DISTRICT"));
    label.addEventListener("click", () => centerOnWard(district.id));
    $("world-labels").append(label);
    wardLabelNodes.set(district.id, label);
  }
  function updateLabels(axes) {
    for (const entry of scene.dossiers) {
      const label = labelNodes.get(entry.id);
      const [x, y] = worldToScreen([entry.pin[0], entry.roof_height + 6, entry.pin[2]], axes);
      const visible = camera.zoom < 550
        && x >= 280 && x <= canvas.clientWidth - 325
        && y >= 85 && y <= canvas.clientHeight - 82;
      label.hidden = !visible;
      if (visible) {
        label.style.left = x + "px";
        label.style.top = y + "px";
      }
    }
    for (const district of scene.districts) {
      const label = wardLabelNodes.get(district.id);
      const midpoint = [
        (district.bounds[0] + district.bounds[2]) / 2,
        0,
        (district.bounds[1] + district.bounds[3]) / 2,
      ];
      const [x, y] = worldToScreen(midpoint, axes);
      const visible = camera.zoom >= 680
        && x >= 285 && x <= canvas.clientWidth - 325
        && y >= 95 && y <= canvas.clientHeight - 90;
      label.hidden = !visible;
      label.classList.toggle("active", district.id === selectedWard);
      if (visible) {
        label.style.left = x + "px";
        label.style.top = y + "px";
      }
    }
  }

  $("close-dossier").addEventListener("click", () => selectBusiness(0));
  $("focus-property").addEventListener("click", () => focusOnProperty(selectedId));
  $("reset-camera").addEventListener("click", () => {
    camera.yaw = Math.PI / 4;
    camera.elevation = 0.89;
    centerOnWard(scene.districts[0].id);
  });

  let dragging = null;
  canvas.addEventListener("contextmenu", (event) => event.preventDefault());
  canvas.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 && event.button !== 2) return;
    dragging = { x: event.clientX, y: event.clientY, startX: event.clientX, startY: event.clientY, orbit: event.button === 2 || event.shiftKey, moved: false };
    canvas.setPointerCapture(event.pointerId);
  });
  canvas.addEventListener("pointermove", (event) => {
    if (!dragging) return;
    const dx = event.clientX - dragging.x;
    const dy = event.clientY - dragging.y;
    if (Math.abs(event.clientX - dragging.startX) + Math.abs(event.clientY - dragging.startY) > 5) {
      dragging.moved = true;
    }
    if (dragging.moved) {
      canvas.classList.add("is-dragging");
      if (dragging.orbit) {
        camera.yaw += dx * 0.008;
        camera.elevation = Math.max(0.35, Math.min(1.30, camera.elevation - dy * 0.005));
      } else {
        const axes = basis();
        const before = groundFromScreen(event.clientX - canvas.getBoundingClientRect().left, event.clientY - canvas.getBoundingClientRect().top, axes);
        const after = groundFromScreen(event.clientX - dx - canvas.getBoundingClientRect().left, event.clientY - dy - canvas.getBoundingClientRect().top, axes);
        camera.x += after[0] - before[0];
        camera.z += after[1] - before[1];
      }
      requestDraw();
    }
    dragging.x = event.clientX;
    dragging.y = event.clientY;
  });
  function endDrag(event) {
    if (!dragging) return;
    const wasClick = !dragging.moved && event.button === 0;
    dragging = null;
    canvas.classList.remove("is-dragging");
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    if (wasClick) {
      try { pick(event.clientX, event.clientY); } catch (error) { fail("Selection failed: " + error.message); }
    }
  }
  canvas.addEventListener("pointerup", endDrag);
  canvas.addEventListener("pointercancel", () => {
    dragging = null;
    canvas.classList.remove("is-dragging");
  });
  canvas.addEventListener("wheel", (event) => {
    event.preventDefault();
    const rect = canvas.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    const before = groundFromScreen(x, y, basis());
    camera.zoom = Math.max(55, Math.min(6000, camera.zoom * Math.exp(event.deltaY * 0.0011)));
    const after = groundFromScreen(x, y, basis());
    camera.x += before[0] - after[0];
    camera.z += before[1] - after[1];
    requestDraw();
  }, { passive: false });
  window.addEventListener("keydown", (event) => {
    if (event.target instanceof HTMLInputElement) return;
    const step = camera.zoom * 0.12;
    if (event.key === "Escape") selectBusiness(0);
    else if (event.key === "q" || event.key === "Q") camera.yaw -= 0.16;
    else if (event.key === "e" || event.key === "E") camera.yaw += 0.16;
    else if (event.key === "+" || event.key === "=") camera.zoom *= 0.82;
    else if (event.key === "-") camera.zoom *= 1.22;
    else if (event.key === "ArrowLeft" || event.key === "a") camera.x -= step;
    else if (event.key === "ArrowRight" || event.key === "d") camera.x += step;
    else if (event.key === "ArrowUp" || event.key === "w") camera.z -= step;
    else if (event.key === "ArrowDown" || event.key === "s") camera.z += step;
    else return;
    camera.zoom = Math.max(55, Math.min(6000, camera.zoom));
    event.preventDefault();
    requestDraw();
  });
  window.addEventListener("resize", requestDraw);
  canvas.addEventListener("webglcontextlost", (event) => {
    event.preventDefault();
    fail("The WebGL context was lost. Reload the preview to recreate its GPU buffers.");
  });

  // Small, read-only inspection surface for headless renderer checks.
  window.crimocracyCityPreview = Object.freeze({
    stats: () => ({ ...visibleStats, uploadedDistricts, selectedId, camera: { ...camera } }),
    selectBusiness: (id) => selectBusiness(Number(id) >>> 0),
    focusOnProperty: (id) => focusOnProperty(Number(id) >>> 0),
    showWard: (id) => centerOnWard(Number(id)),
    groundAt: (x, y) => groundFromScreen(Number(x), Number(y), basis()),
    projectProperty: (id) => {
      const entry = properties.get(Number(id) >>> 0);
      return entry ? worldToScreen([entry.pin[0], entry.roof_height, entry.pin[2]], basis()) : null;
    },
  });
  requestDraw();
})();
