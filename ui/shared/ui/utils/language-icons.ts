/**
 * Map a programming language name, possibly output by an LLM (not super deterministic),
 * to a devicon (https://devicon.dev) icon name.
 */
export const mapLanguageToDevIcon = (language: string) => {
  return languageAliases[language.toLowerCase()];
};

/**
 * All icon names available in devicon 2.17.0.
 * Declared as a static set rather than imported from devicon/devicon.json
 * to avoid pulling ~300kb of metadata into the client bundle.
 * A unit test asserts this set stays in sync with the package.
 */
// prettier-ignore
export const DEVICON_NAMES = new Set([
  "aarch64", "adonisjs", "aerospike", "aframe", "aftereffects", "akka", "algolia", "almalinux",
  "alpinejs", "amazonwebservices", "anaconda", "android", "androidstudio", "angular", "angularjs", "angularmaterial",
  "ansible", "ansys", "antdesign", "apache", "apacheairflow", "apachekafka", "apachespark", "apex",
  "apl", "apollographql", "appcelerator", "apple", "appwrite", "archlinux", "arduino", "argocd",
  "artixlinux", "astro", "atom", "awk", "axios", "azure", "azuredevops", "azuresqldatabase",
  "babel", "babylonjs", "backbonejs", "ballerina", "bamboo", "bash", "bazel", "beats",
  "behance", "bevyengine", "biome", "bitbucket", "blazor", "blender", "bootstrap", "bower",
  "browserstack", "bulma", "bun", "c", "cairo", "cakephp", "canva", "capacitor",
  "carbon", "cassandra", "centos", "ceylon", "chakraui", "chartjs", "chrome", "circleci",
  "clarity", "clickhouse", "clion", "clojure", "clojurescript", "cloudflare", "cloudflareworkers", "cloudrun",
  "cmake", "cobol", "codeac", "codecov", "codeigniter", "codepen", "coffeescript", "composer",
  "confluence", "consul", "contao", "corejs", "cosmosdb", "couchbase", "couchdb", "cpanel",
  "cplusplus", "crystal", "csharp", "css3", "cucumber", "cypressio", "d3js", "dart",
  "datadog", "datagrip", "dataspell", "datatables", "dbeaver", "debian", "delphi", "denojs",
  "detaspace", "devicon", "digitalocean", "discloud", "discordjs", "django", "djangorest", "docker",
  "doctrine", "dot-net", "dotnetcore", "dovecot", "dreamweaver", "dropwizard", "drupal", "duckdb",
  "dyalog", "dynamodb", "dynatrace", "eclipse", "ecto", "elasticsearch", "electron", "eleventy",
  "elixir", "elm", "emacs", "embeddedc", "ember", "entityframeworkcore", "envoy", "erlang",
  "eslint", "expo", "express", "facebook", "fastapi", "fastify", "faunadb", "feathersjs",
  "fedora", "fiber", "figma", "filamentphp", "filezilla", "firebase", "firebird", "firefox",
  "flask", "flutter", "forgejo", "fortran", "foundation", "framermotion", "framework7", "fsharp",
  "fusion", "gardener", "gatling", "gatsby", "gazebo", "gcc", "gentoo", "ghost",
  "gimp", "git", "gitbook", "github", "githubactions", "githubcodespaces", "gitkraken", "gitlab",
  "gitpod", "gitter", "gleam", "glitch", "go", "godot", "goland", "google",
  "googlecloud", "googlecolab", "gradle", "grafana", "grails", "graphql", "groovy", "grpc",
  "grunt", "gulp", "hadoop", "handlebars", "harbor", "hardhat", "harvester", "haskell",
  "haxe", "helm", "heroku", "hibernate", "homebrew", "hoppscotch", "html5", "htmx",
  "hugo", "hyperv", "ie10", "ifttt", "illustrator", "inertiajs", "influxdb", "inkscape",
  "insomnia", "intellij", "ionic", "jaegertracing", "jamstack", "jasmine", "java", "javascript",
  "jeet", "jekyll", "jenkins", "jest", "jetbrains", "jetpackcompose", "jhipster", "jira",
  "jiraalign", "jquery", "json", "jule", "julia", "junit", "jupyter", "k3os",
  "k3s", "k6", "kaggle", "kaldi", "kalilinux", "karatelabs", "karma", "kdeneon",
  "keras", "kibana", "knexjs", "knockout", "kotlin", "krakenjs", "ktor", "kubeflow",
  "kubernetes", "labview", "laminas", "laravel", "laraveljetstream", "latex", "leetcode", "less",
  "libgdx", "linkedin", "linux", "linuxmint", "liquibase", "livewire", "llvm", "lodash",
  "logstash", "love2d", "lua", "lumen", "magento", "mapbox", "mariadb", "markdown",
  "materializecss", "materialui", "matlab", "matplotlib", "mattermost", "maven", "maya", "memcached",
  "mercurial", "meteor", "microsoftsqlserver", "minitab", "mithril", "mobx", "mocha", "modx",
  "moleculer", "mongodb", "mongoose", "monogame", "moodle", "msdos", "mysql", "nano",
  "nats", "neo4j", "neovim", "nestjs", "netbeans", "netbox", "netlify", "networkx",
  "newrelic", "nextjs", "nginx", "ngrok", "ngrx", "nhibernate", "nim", "nimble",
  "nixos", "nodejs", "nodemon", "nodered", "nodewebkit", "nomad", "norg", "notion",
  "npm", "npss", "nuget", "numpy", "nuxt", "nuxtjs", "oauth", "objectivec",
  "ocaml", "ohmyzsh", "okta", "openal", "openapi", "opencl", "opencv", "opengl",
  "openstack", "opensuse", "opentelemetry", "opera", "oracle", "ory", "p5js", "packer",
  "pandas", "passport", "perl", "pfsense", "phalcon", "phoenix", "photonengine", "photoshop",
  "php", "phpstorm", "pixijs", "playwright", "plotly", "pm2", "pnpm", "podman",
  "poetry", "polygon", "portainer", "postcss", "postgresql", "postman", "powershell", "premierepro",
  "primeng", "prisma", "processing", "processwire", "prolog", "prometheus", "protractor", "proxmox",
  "pug", "pulsar", "pulumi", "puppeteer", "purescript", "putty", "pycharm", "pypi",
  "pyscript", "pytest", "python", "pytorch", "qodana", "qt", "qtest", "quarkus",
  "quasar", "qwik", "r", "rabbitmq", "racket", "radstudio", "rails", "railway",
  "rancher", "raspberrypi", "reach", "react", "reactbootstrap", "reactnative", "reactnavigation", "reactrouter",
  "readthedocs", "realm", "rect", "redhat", "redis", "redux", "reflex", "remix",
  "renpy", "replit", "rexx", "rider", "rocksdb", "rockylinux", "rollup", "ros",
  "rspec", "rstudio", "ruby", "rubymine", "rust", "rxjs", "safari", "salesforce",
  "sanity", "sass", "scala", "scalingo", "scikitlearn", "sdl", "selenium", "sema",
  "sentry", "sequelize", "shopware", "shotgrid", "sketch", "slack", "socketio", "solidity",
  "solidjs", "sonarqube", "sourceengine", "sourcetree", "spack", "spicedb", "splunk", "spring",
  "spss", "spyder", "sqlalchemy", "sqldeveloper", "sqlite", "ssh", "stackblitz", "stackoverflow",
  "stata", "stenciljs", "storybook", "streamlit", "styledcomponents", "stylus", "subversion", "sulu",
  "supabase", "surrealdb", "svelte", "svgo", "swagger", "swift", "swiper", "symfony",
  "tailwindcss", "talos", "tauri", "teleport", "tensorflow", "terraform", "terramate", "tex",
  "thealgorithms", "threedsmax", "threejs", "thymeleaf", "titaniumsdk", "tmux", "tomcat", "tortoisegit",
  "towergit", "traefikmesh", "traefikproxy", "travis", "trello", "trpc", "turbo", "twilio",
  "twitter", "typescript", "typo3", "ubuntu", "unifiedmodelinglanguage", "unity", "unix", "unrealengine",
  "uwsgi", "v8", "vaadin", "vagrant", "vala", "vault", "veevalidate", "vercel",
  "vertx", "vim", "visualbasic", "visualstudio", "vite", "vitejs", "vitess", "vitest",
  "vscode", "vscodium", "vsphere", "vuejs", "vuestorefront", "vuetify", "vulkan", "vyper",
  "waku", "wasm", "web3js", "webflow", "webgpu", "weblate", "webpack", "webstorm",
  "windows11", "windows8", "wolfram", "woocommerce", "wordpress", "xamarin", "xcode", "xd",
  "xml", "yaml", "yarn", "yii", "yugabytedb", "yunohost", "zend", "zig",
  "zsh", "zustand",
]);

/**
 * Languages that have alternative Lucide icons in LanguageIcon component.
 * Keep in sync with alternativeLanguageMapping in language-icon.tsx.
 */
const ALTERNATIVE_ICON_LANGUAGES = new Set(["sql"]);

/**
 * Check whether a language or file extension has a corresponding icon.
 * Checks the alias mapping, the native devicon catalog, and alternative Lucide icons.
 */
export const hasDevIcon = (language: string): boolean => {
  const lower = language.toLowerCase();

  // Check if there's an alternative Lucide icon for this language
  if (ALTERNATIVE_ICON_LANGUAGES.has(lower)) {
    return true;
  }

  const alias = languageAliases[lower];
  if (alias !== undefined) {
    return DEVICON_NAMES.has(alias);
  }
  return DEVICON_NAMES.has(lower);
};

const languageAliases: Record<string, string> = {
  assembly: "aarch64",
  aws: "amazonwebservices",
  py: "python",
  python3: "python",
  sage: "python",
  js: "javascript",
  node: "javascript",
  nodejs: "javascript",
  nodets: "typescript",
  cpp: "cplusplus",
  "c++": "cplusplus",
  cc: "cplusplus",
  cs: "csharp",
  "c#": "csharp",
  php3: "php",
  php4: "php",
  php5: "php",
  php6: "php",
  rb: "ruby",
  kt: "kotlin",
  golang: "go",
  rs: "rust",
  ts: "typescript",
  tsx: "typescript",
  ex: "elixir",
  exs: "elixir",
  clj: "clojure",
  hs: "haskell",
  erl: "erlang",
  gcp: "googlecloud",
  jl: "julia",
  pl: "perl",
  objc: "objectivec",
  mm: "objectivec",
  m: "matlab",
  xhtml: "html5",
  html: "html5",
  sh: "bash",
  shell: "bash",
  ps1: "powershell",
  psm1: "powershell",
  coffee: "coffeescript",
  fs: "fsharp",
  ml: "ocaml",
  rkt: "racket",
  commonlisp: "lisp",
  f77: "fortran",
  f90: "fortran",
  f95: "fortran",
  sol: "solidity",
  vy: "vyper",
  cr: "crystal",
  re: "reasonml",
  purs: "purescript",
  idr: "idris",
  dosbatch: "batch",
  winbatch: "batch",
  vbs: "vbscript",
  wat: "wasm",
  yml: "yaml",
  md: "markdown",
  delphi: "pascal",
  asm: "aarch64",
  gql: "graphql",
  jsx: "javascript",
  scss: "sass",
  css: "css3",
  dockerfile: "docker",
  fish: "bash",
  json5: "json",
  jsonc: "json",
  ipython: "python",
  sql: "sql",
  plsql: "postgresql",
  tsql: "postgresql",
  vlang: "v",
  dlang: "d",
  vue: "vuejs",
  svelte: "svelte",
  solid: "solidjs",
  solidity: "solidity",
};

/**
 * Check if a language should display its name and icon in the header.
 * Returns true for all languages except plain "text".
 */
export const isKnownLanguage = (language: string): boolean => {
  return language.toLowerCase() !== "text";
};
