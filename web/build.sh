#!/bin/bash
set -e
if [ "$1" == "--watch" ]; then
    echo "Watching OpenSanctuary Frontend with Bun..."
    bun build src/main.ts --outdir dist --entry-naming "bundle.[ext]" --target browser --sourcemap=external --watch &
    bun build src/live_output.ts --outdir dist --entry-naming "live_output.[ext]" --target browser --sourcemap=external --watch &
    wait
else
    echo "Bundling OpenSanctuary Frontend (Zero-Config TypeScript via Bun)..."
    bun build src/main.ts --outdir dist --entry-naming "bundle.[ext]" --target browser --sourcemap=external --minify
    bun build src/live_output.ts --outdir dist --entry-naming "live_output.[ext]" --target browser --sourcemap=external --minify
    echo "Build complete! Output: dist/bundle.js and dist/live_output.js"
fi
