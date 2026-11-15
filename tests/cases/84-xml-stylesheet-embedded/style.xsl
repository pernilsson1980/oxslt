<?xml version="1.0"?>
<!-- href="#id": the element with that ID is the stylesheet (copied with the
     namespace declarations of its ancestors), and transforms the whole
     document, itself included. -->
<!DOCTYPE doc [ <!ATTLIST xsl:stylesheet id ID #IMPLIED> ]>
<?xml-stylesheet type="text/xml" href="#style"?>
<doc xmlns:xsl="http://www.w3.org/1999/XSL/Transform" xmlns:e="urn:extra">
  <e:data>embedded</e:data>
  <xsl:stylesheet id="style" version="1.0">
    <xsl:output method="text"/>
    <xsl:template match="/"><xsl:value-of select="concat(doc/e:data, ' ', count(//xsl:template))"/></xsl:template>
  </xsl:stylesheet>
</doc>
