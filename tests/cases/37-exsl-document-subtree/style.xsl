<?xml version="1.0"?>
<!-- writesubtree allows writes under a path prefix; hrefs are relative to
     the -o file. -->
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:exsl="http://exslt.org/common" extension-element-prefixes="exsl">
  <xsl:template match="/">
    <main>
      <exsl:document href="beside-main.xml"><inside/></exsl:document>
      <exsl:document href="sub/made.xml"><inside-new-dir/></exsl:document>
      <exsl:document href="../outside.xml"><outside/></exsl:document>
    </main>
  </xsl:template>
</xsl:stylesheet>
