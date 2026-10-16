const root = document.documentElement;
const demoFrame = document.getElementById("i3rs-demo");
const demoStatus = document.getElementById("demo-status");
let demoLoaded = false;
let demoLoadStarted = false;

const setScrollY = () => {
  root.style.setProperty("--scroll-y", String(window.scrollY));
};

setScrollY();
window.addEventListener("scroll", setScrollY, { passive: true });

const reveals = document.querySelectorAll(".reveal");
const revealObserver = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      if (entry.isIntersecting) {
        entry.target.classList.add("is-visible");
        revealObserver.unobserve(entry.target);
      }
    }
  },
  { rootMargin: "0px 0px -12% 0px", threshold: 0.18 },
);

for (const element of reveals) {
  revealObserver.observe(element);
}

function setDemoStatus(message) {
  demoStatus.textContent = message;
}

function ensureDemoFrame() {
  const demoSrc = demoFrame.dataset.src;
  if (demoSrc && demoFrame.getAttribute("src") !== demoSrc) {
    setDemoStatus("Starting demo");
    demoFrame.setAttribute("src", demoSrc);
  }
}

async function waitForFrameRuntime(frameWindow) {
  for (let attempt = 0; attempt < 160; attempt += 1) {
    if (typeof frameWindow.i3rsLoadSession === "function") {
      return true;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }

  return false;
}

async function autoloadVirLap() {
  if (demoLoaded || demoLoadStarted) {
    return;
  }

  demoLoadStarted = true;
  const frameWindow = demoFrame.contentWindow;
  if (!frameWindow) {
    setDemoStatus("Open the demo full screen");
    return;
  }

  setDemoStatus("Starting demo");
  const ready = await waitForFrameRuntime(frameWindow);
  if (!ready) {
    setDemoStatus("Demo startup timed out");
    return;
  }

  try {
    await frameWindow.i3rsLoadBundledSample();
    demoLoaded = true;
  } catch (error) {
    console.error(error);
    setDemoStatus("Demo could not load the VIR lap");
  }
}

demoFrame.addEventListener("load", () => {
  if (!demoFrame.getAttribute("src")?.includes("/demo/")) {
    return;
  }

  try {
    const frameWindow = demoFrame.contentWindow;
    frameWindow.i3rsOnLoadStateChange = (state) => {
      if (state?.state === "success") {
        demoLoaded = true;
        setDemoStatus(state.message || "VIR_LAP loaded");
      } else if (state?.message) {
        setDemoStatus(state.message);
      }
    };
    void autoloadVirLap();
  } catch {
    setDemoStatus("Open the demo full screen to load the sample");
  }
});

function loadDemoFromHash() {
  if (window.location.hash === "#demo") {
    ensureDemoFrame();
  }
}

window.addEventListener("hashchange", loadDemoFromHash);
loadDemoFromHash();

const demoObserver = new IntersectionObserver(
  (entries) => {
    if (entries.some((entry) => entry.isIntersecting)) {
      ensureDemoFrame();
      demoObserver.disconnect();
    }
  },
  { rootMargin: "240px 0px", threshold: 0.01 },
);

demoObserver.observe(demoFrame);

window.addEventListener("message", (event) => {
  if (event.source === demoFrame.contentWindow && event.data?.type === "i3rs-status") {
    setDemoStatus(event.data.message);
  }
});
