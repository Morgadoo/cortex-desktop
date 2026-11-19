import semver from 'semver';

const [minimum, release] = process.argv.slice(2);
if (!minimum) process.exit(0);

function strictVersion(value, label) {
    const parsed = semver.parse(value);
    const canonical = parsed && parsed.version + (parsed.build.length ? `+${parsed.build.join('.')}` : '');
    if (!parsed || value !== canonical) {
        throw new Error(`${label} '${value}' is not strict semver`);
    }
    return parsed;
}

try {
    const min = strictVersion(minimum, 'CORTEX_MINIMUM_SUPPORTED_VERSION');
    const version = strictVersion(release, 'version');
    if (semver.compareBuild(min, version) > 0) {
        throw new Error(`CORTEX_MINIMUM_SUPPORTED_VERSION '${minimum}' > version '${release}'`);
    }
} catch (error) {
    console.error(`make-update-manifest: ${error.message}`);
    process.exitCode = 1;
}
