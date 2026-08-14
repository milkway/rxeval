// Ask JavaRosa what each expression means, and write the answers down.
//
//   javac -cp 'lib/*' -d out scripts/JavarosaOracle.java
//   java -cp 'lib/*:out' JavarosaOracle <oracle-dir> <version> > javarosa-expected.json
//
// JavaRosa is the engine inside ODK Collect and KoboCollect — the code that
// evaluated these forms on the tablets whose submissions this server
// stores. Enketo answers the same questions from the web side; where the
// two disagree, this is the one whose answer already shaped the data.
//
// Fetch the jars with any Maven, e.g.
//   docker run --rm -v "$PWD":/w -w /w maven:3-eclipse-temurin-21 \
//     mvn -q -B dependency:copy-dependencies -DoutputDirectory=lib

import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import org.javarosa.core.model.condition.EvaluationContext;
import org.javarosa.core.model.data.IAnswerData;
import org.javarosa.core.model.instance.FormInstance;
import org.javarosa.xform.parse.XFormParser;
import org.javarosa.core.model.instance.TreeElement;
import org.javarosa.xpath.XPathNodeset;
import org.javarosa.xpath.expr.XPathExpression;
import org.javarosa.xpath.parser.XPathSyntaxException;
import org.javarosa.xpath.XPathParseTool;

public class JavarosaOracle {

    /** The instance being queried, for resolving a nodeset's references. */
    private static FormInstance INSTANCE;

    public static void main(String[] args) throws Exception {
        Path dir = Path.of(args.length > 0 ? args[0] : ".");
        String instanceXml = Files.readString(dir.resolve("instance.xml"));
        List<String> corpus = new ArrayList<>();
        for (String line : Files.readAllLines(dir.resolve("corpus.txt"))) {
            String trimmed = line.trim();
            if (!trimmed.isEmpty() && !trimmed.startsWith("#")) corpus.add(trimmed);
        }

        // Parse the form, then load the data into it — the order Collect
        // uses. Evaluating against a bare instance instead would measure
        // the harness: without a definition JavaRosa cannot tell that
        // `resident` repeats, so every element takes the same multiplicity
        // and a positional predicate matches nothing.
        // JavaRosa keeps its answer-parsing factory in a static that only
        // this registration fills. Without it, restoring saved data dies on
        // the first typed answer.
        new org.javarosa.model.xform.XFormsModule().registerModule();

        // The two-reader constructor is exactly this: a form, and saved
        // data to restore into it.
        org.javarosa.core.model.FormDef form = new XFormParser(
                new java.io.StringReader(Files.readString(dir.resolve("form.xml"))),
                new java.io.StringReader(instanceXml))
                .parse();
        FormInstance instance = form.getMainInstance();
        INSTANCE = instance;
        // The context node is the instance root, which is where a bind's
        // absolute path starts from.
        EvaluationContext context =
                new EvaluationContext(instance, new java.util.HashMap<>());
        context = new EvaluationContext(context, instance.getRoot().getRef());

        PrintStream out = new PrintStream(System.out, true, StandardCharsets.UTF_8);
        out.println("{");
        // Which build answered, so a fixture that moves can be explained.
        // The jar carries no Implementation-Version, so the caller passes
        // the coordinate it resolved.
        String version = args.length > 1 ? args[1] : "unknown";
        out.println("  \"$meta\": {\"engine\": \"javarosa\", \"version\": "
                + quote(version) + "},");
        for (int i = 0; i < corpus.size(); i++) {
            String expression = corpus.get(i);
            String entry;
            try {
                XPathExpression parsed = XPathParseTool.parseXPath(expression);
                Object value = parsed.eval(instance, context);
                entry = describe(value);
            } catch (XPathSyntaxException e) {
                entry = refusal("parse", e);
            } catch (Exception e) {
                // A refusal is an answer too. If one engine refuses where
                // another invents a value, that is the divergence most
                // worth knowing about.
                entry = refusal("eval", e);
            }
            out.print("  " + quote(expression) + ": " + entry);
            out.println(i + 1 < corpus.size() ? "," : "");
        }
        out.println("}");
    }

    private static String describe(Object value) {
        if (value instanceof Boolean b) {
            return "{\"type\": \"boolean\", \"value\": " + b + ", \"via\": \"javarosa\"}";
        }
        if (value instanceof Double d) {
            String number;
            if (d.isNaN()) number = "\"NaN\"";
            else if (d.isInfinite()) number = d > 0 ? "\"Infinity\"" : "\"-Infinity\"";
            else number = String.valueOf((double) d);
            return "{\"type\": \"number\", \"value\": " + number + ", \"via\": \"javarosa\"}";
        }
        if (value instanceof XPathNodeset nodeset) {
            // Compared by the values the nodes hold, in document order:
            // node identity means nothing across implementations, and the
            // values are what a form goes on to use.
            //
            // Read through the reference rather than getValAt, which
            // unpacks to the XPath type — an integer answer comes back as
            // 42.0 that way, and the string a form would see is "42".
            StringBuilder items = new StringBuilder();
            for (int i = 0; i < nodeset.size(); i++) {
                if (i > 0) items.append(", ");
                String text = "";
                Object resolved = INSTANCE.resolveReference(nodeset.getRefAt(i));
                if (resolved instanceof TreeElement element && element.getValue() != null) {
                    text = element.getValue().getDisplayText();
                }
                items.append(quote(text));
            }
            return "{\"type\": \"nodeset\", \"value\": [" + items + "], \"via\": \"javarosa\"}";
        }
        return "{\"type\": \"string\", \"value\": " + quote(stringOf(value))
                + ", \"via\": \"javarosa\"}";
    }

    private static String stringOf(Object value) {
        if (value instanceof IAnswerData answer) {
            Object display = answer.getValue();
            return display == null ? "" : String.valueOf(display);
        }
        return String.valueOf(value);
    }

    private static String refusal(String stage, Exception e) {
        String message = e.getMessage() == null ? e.getClass().getSimpleName() : e.getMessage();
        return "{\"type\": \"error\", \"value\": " + quote(stage + ": " + message)
                + ", \"via\": \"javarosa\"}";
    }

    private static String quote(String text) {
        StringBuilder out = new StringBuilder("\"");
        for (char c : text.toCharArray()) {
            switch (c) {
                case '"' -> out.append("\\\"");
                case '\\' -> out.append("\\\\");
                case '\n' -> out.append("\\n");
                case '\r' -> out.append("\\r");
                case '\t' -> out.append("\\t");
                default -> {
                    if (c < 0x20) out.append(String.format("\\u%04x", (int) c));
                    else out.append(c);
                }
            }
        }
        return out.append('"').toString();
    }
}
