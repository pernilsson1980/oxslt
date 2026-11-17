<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:func="http://exslt.org/functions" xmlns:my="urn:my" extension-element-prefixes="func">
<func:function name="my:which"><func:result select="'imported'"/></func:function>
<func:function name="my:only-imported"><func:result select="'from import'"/></func:function>
</xsl:stylesheet>
