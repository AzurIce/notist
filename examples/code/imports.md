# Code member imports in Markdown

#import "./lib.not": decorate as mark, tag;
#import "./labels.not": *;

#mark[The same renamed function is available in Markdown.]

#tag("Code retains the defining module scope.")

#label("Wildcard import of a public function.", true)
