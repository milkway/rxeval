# The oracle generators

Two scripts, one per reference engine. They put the same corpus to each and
write the answers down; the answers are committed, so the Rust tests compare
against them without needing Node or a JVM.

    node scripts/openrosa-oracle.mjs tests/oracle > tests/oracle/expected.json
    javac -cp 'lib/*' -d out scripts/JavarosaOracle.java
    java  -cp 'lib/*:out' JavarosaOracle tests/oracle 5.1.0 > tests/oracle/javarosa-expected.json

Regenerate when the corpus changes, and read the diff: a line that moves is
either a bug fixed upstream or a case where the two implementations were
never going to agree.

The JavaRosa jars are not committed. Fetch them with any Maven:

    mvn -f pom.xml dependency:copy-dependencies -DoutputDirectory=lib

with `com.github.getodk:javarosa:v5.1.0` from jitpack. The Node side needs

    npm install openrosa-xpath-evaluator jsdom
